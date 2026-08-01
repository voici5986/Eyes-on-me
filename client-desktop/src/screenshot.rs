use std::{io::ErrorKind, path::Path, time::Duration};

use anyhow::{Context, bail};
use image::{DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use tokio::{fs, process::Command, time::timeout};
use uuid::Uuid;

use crate::{
    config::{ScreenshotConfig, ScreenshotDisplay, ScreenshotFormat},
    event::ActivityEnvelope,
};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct CapturedScreenshot {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub async fn capture(
    config: &ScreenshotConfig,
    event: &ActivityEnvelope,
) -> anyhow::Result<CapturedScreenshot> {
    let path = std::env::temp_dir().join(format!("eyes-on-me-{}.png", Uuid::new_v4()));
    let result = capture_to_path(&path, config.display, event.payload.app.pid).await;
    let raw = match result {
        Ok(()) => fs::read(&path)
            .await
            .context("failed to read captured screenshot"),
        Err(err) => Err(err),
    };
    let _ = fs::remove_file(&path).await;
    encode(raw?, config)
}

fn encode(bytes: Vec<u8>, config: &ScreenshotConfig) -> anyhow::Result<CapturedScreenshot> {
    let mut image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .context("captured screenshot was not valid PNG")?;
    let (width, height) = image.dimensions();
    if width > config.max_width {
        let next_height =
            ((u64::from(height) * u64::from(config.max_width)) / u64::from(width)).max(1) as u32;
        image = image.resize(
            config.max_width,
            next_height,
            image::imageops::FilterType::Triangle,
        );
    }

    match config.format {
        ScreenshotFormat::Png => encode_png(image),
        ScreenshotFormat::Jpeg => encode_jpeg(image, config.jpeg_quality),
    }
}

fn encode_png(image: DynamicImage) -> anyhow::Result<CapturedScreenshot> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    image.write_to(&mut cursor, ImageFormat::Png)?;
    Ok(CapturedScreenshot {
        mime_type: "image/png".to_string(),
        bytes: cursor.into_inner(),
    })
}

fn encode_jpeg(image: DynamicImage, quality: u8) -> anyhow::Result<CapturedScreenshot> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, quality).encode_image(&image)?;
    Ok(CapturedScreenshot {
        mime_type: "image/jpeg".to_string(),
        bytes,
    })
}

#[cfg(target_os = "macos")]
async fn capture_to_path(
    path: &Path,
    display: ScreenshotDisplay,
    pid: Option<u32>,
) -> anyhow::Result<()> {
    let mut command = Command::new("/usr/sbin/screencapture");
    command.arg("-x").arg("-t").arg("png");
    match display {
        ScreenshotDisplay::Active => {
            if let Some(window_id) = pid
                .and_then(|value| i32::try_from(value).ok())
                .and_then(crate::platform::macos_native::frontmost_window_id)
            {
                command.arg("-o").arg("-l").arg(window_id.to_string());
            } else {
                command.arg("-m");
            }
        }
        ScreenshotDisplay::Primary => {
            command.arg("-m");
        }
        ScreenshotDisplay::All => {}
    }
    command.arg(path);
    run_command(&mut command).await
}

#[cfg(target_os = "windows")]
async fn capture_to_path(
    path: &Path,
    _display: ScreenshotDisplay,
    _pid: Option<u32>,
) -> anyhow::Result<()> {
    let script = r#"Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; $i=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($i); $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); $i.Save($args[0],[System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $i.Dispose()"#;
    run_command(
        Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .arg(path),
    )
    .await
}

#[cfg(target_os = "linux")]
async fn capture_to_path(
    path: &Path,
    _display: ScreenshotDisplay,
    _pid: Option<u32>,
) -> anyhow::Result<()> {
    let mut attempts = Vec::new();
    let mut commands = [
        command("gnome-screenshot", &["-f"], path),
        command("spectacle", &["-b", "-n", "-o"], path),
        command("grim", &[], path),
        command("scrot", &[], path),
    ];
    for command in &mut commands {
        match run_command(command).await {
            Ok(()) => return Ok(()),
            Err(err) => attempts.push(err.to_string()),
        }
    }
    bail!(
        "no supported Linux screenshot provider succeeded: {}",
        attempts.join("; ")
    )
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
async fn capture_to_path(
    _path: &Path,
    _display: ScreenshotDisplay,
    _pid: Option<u32>,
) -> anyhow::Result<()> {
    bail!("screenshots are not supported on this platform")
}

#[cfg(target_os = "linux")]
fn command(program: &str, arguments: &[&str], path: &Path) -> Command {
    let mut command = Command::new(program);
    command.args(arguments).arg(path);
    command
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
pub fn permission_status() -> &'static str {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        "granted"
    } else {
        "missing"
    }
}

#[cfg(not(target_os = "macos"))]
pub fn permission_status() -> &'static str {
    "not_required"
}

async fn run_command(command: &mut Command) -> anyhow::Result<()> {
    let output = match timeout(CAPTURE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) if err.kind() == ErrorKind::NotFound => {
            bail!("screenshot command is not installed")
        }
        Ok(Err(err)) => return Err(err.into()),
        Err(_) => bail!("screenshot command timed out"),
    };
    if !output.status.success() {
        bail!(
            "screenshot command exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
