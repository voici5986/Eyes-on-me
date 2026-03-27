use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let web_dist_dir = manifest_dir.join("../web/dist");

    println!("cargo:rerun-if-changed={}", web_dist_dir.display());

    let embed_dir = if has_built_web_dist(&web_dist_dir) {
        web_dist_dir
    } else {
        create_placeholder_web_dist(&manifest_dir)
    };

    println!(
        "cargo:rustc-env=AMI_OKAY_EMBED_WEB_DIST={}",
        embed_dir.display()
    );
}

fn has_built_web_dist(path: &Path) -> bool {
    path.is_dir() && path.join("index.html").is_file()
}

fn create_placeholder_web_dist(manifest_dir: &Path) -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let placeholder_dir = out_dir.join("embedded-web-dist");
    fs::create_dir_all(&placeholder_dir).expect("create placeholder web dist");

    let placeholder = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Eyes on Me</title>
    <style>
      :root { color-scheme: light; }
      body {
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: #f5f7fb;
        color: #172033;
        font: 16px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      main {
        width: min(640px, calc(100vw - 48px));
        padding: 32px;
        border-radius: 20px;
        background: white;
        box-shadow: 0 20px 50px rgba(23, 32, 51, 0.08);
      }
      code {
        padding: 2px 6px;
        border-radius: 6px;
        background: #eef2ff;
      }
    </style>
  </head>
  <body>
    <main>
      <h1>Eyes on Me web assets are not built yet.</h1>
      <p>Run <code>pnpm build</code> in the <code>web/</code> directory, then rebuild <code>client-server</code>.</p>
      <p>For local development, you can also run the Vite dev server and open <code>http://127.0.0.1:5173</code>.</p>
      <p>Workspace: __WORKSPACE__</p>
    </main>
  </body>
</html>
"#;

    let placeholder = placeholder.replace("__WORKSPACE__", &manifest_dir.display().to_string());
    fs::write(placeholder_dir.join("index.html"), placeholder).expect("write placeholder index");
    placeholder_dir
}
