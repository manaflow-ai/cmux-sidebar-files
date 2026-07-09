use std::{env, path::Path};

use anyhow::{Result, anyhow, bail};
use cmux_client::CmuxClient;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy)]
pub struct FocusedTarget {
    pub pane: u64,
    pub surface: u64,
}

pub fn focused_target(client: &mut CmuxClient) -> Result<(FocusedTarget, Option<String>)> {
    client.identify()?;
    let tree = client.list_workspaces()?;
    let workspace = tree
        .workspaces
        .iter()
        .find(|workspace| workspace.active)
        .ok_or_else(|| anyhow!("cmux has no active workspace"))?;
    let screen = workspace
        .screens
        .iter()
        .find(|screen| screen.active)
        .ok_or_else(|| anyhow!("cmux has no active screen"))?;
    let pane = screen
        .panes
        .iter()
        .find(|pane| pane.id == screen.active_pane)
        .ok_or_else(|| anyhow!("cmux active pane {} is missing", screen.active_pane))?;
    let tab = pane
        .tabs
        .get(pane.active_tab)
        .ok_or_else(|| anyhow!("cmux focused pane has no active tab"))?;
    let target = FocusedTarget {
        pane: pane.id,
        surface: tab.surface,
    };

    let cwd = if tab.kind == "pty" {
        process_cwd(client, tab.surface).ok().flatten()
    } else {
        None
    };
    Ok((target, cwd))
}

fn process_cwd(client: &mut CmuxClient, surface: u64) -> Result<Option<String>> {
    let mut request = Map::new();
    request.insert("cmd".into(), Value::from("process-info"));
    request.insert("surface".into(), Value::from(surface));
    let data = raw_data(client, request)?;
    Ok(data
        .get("cwd")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

pub fn send_cd(client: &mut CmuxClient, target: FocusedTarget, directory: &Path) -> Result<()> {
    let quoted = shell_single_quote(&directory.to_string_lossy());
    client.send(target.surface, Some(&format!("cd {quoted}\n")), None)?;
    Ok(())
}

pub fn open_editor(client: &mut CmuxClient, target: FocusedTarget, path: &Path) -> Result<()> {
    let editor = env::var("EDITOR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let mut request = Map::new();
    request.insert("cmd".into(), Value::from("run"));
    request.insert("pane".into(), Value::from(target.pane));
    request.insert(
        "cwd".into(),
        Value::from(
            path.parent()
                .unwrap_or_else(|| Path::new("/"))
                .to_string_lossy()
                .into_owned(),
        ),
    );
    request.insert(
        "argv".into(),
        Value::Array(vec![
            Value::from(editor),
            Value::from(path.to_string_lossy().into_owned()),
        ]),
    );
    raw_data(client, request)?;
    Ok(())
}

pub fn open_browser(client: &mut CmuxClient, target: FocusedTarget, path: &Path) -> Result<()> {
    client.new_browser_tab(&file_url(path), Some(target.pane), None, None)?;
    Ok(())
}

fn raw_data(client: &mut CmuxClient, request: Map<String, Value>) -> Result<Value> {
    let response = client.send_raw(request)?;
    if response.get("ok") != Some(&Value::Bool(true)) {
        bail!(
            "{}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("cmux command failed")
        );
    }
    Ok(response
        .get("data")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new())))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut url = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(char::from(byte))
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_paths_with_apostrophes() {
        assert_eq!(shell_single_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
    }

    #[test]
    fn creates_percent_encoded_file_url() {
        assert_eq!(
            file_url(Path::new("/tmp/a file#1.md")),
            "file:///tmp/a%20file%231.md"
        );
    }
}
