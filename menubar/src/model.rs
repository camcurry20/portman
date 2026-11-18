use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const DEFAULT_SETTINGS: &str = r#"# Settings for the portman menu bar app.

# Fixed addresses (http://localhost:<port>) that can be pointed at any project.
slots = [5001, 5002]

# What "Start Dev Server" runs. The first entry whose `repo` text appears in the
# project's git origin URL wins; an entry without `repo` matches every project.
# The command runs in the project folder with $PORT set to the project's port.
#
# [[commands]]
# repo = "my-org/my-repo"
# run = "npm run dev -- --port $PORT"
"#;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub port: u16,
    pub directory: Option<PathBuf>,
    pub linked_port: Option<u16>,
}

#[derive(Default, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    projects: BTreeMap<String, Project>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct StartCommand {
    pub repo: Option<String>,
    pub run: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Settings {
    #[serde(default = "default_slots")]
    pub slots: Vec<u16>,
    #[serde(default)]
    pub commands: Vec<StartCommand>,
}

fn default_slots() -> Vec<u16> {
    vec![5001, 5002]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            slots: default_slots(),
            commands: Vec::new(),
        }
    }
}

pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/com.canac.portman")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("menubar.toml")
}

pub fn log_path(project: &str) -> PathBuf {
    data_dir().join("logs").join(format!("{project}.log"))
}

pub fn load_projects() -> BTreeMap<String, Project> {
    std::fs::read_to_string(data_dir().join("registry.toml"))
        .ok()
        .and_then(|text| toml::from_str::<RegistryFile>(&text).ok())
        .map(|file| file.projects)
        .unwrap_or_default()
}

pub fn load_settings() -> Settings {
    let path = settings_path();
    if !path.exists() {
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(&path, DEFAULT_SETTINGS);
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn is_listening(port: u16) -> bool {
    ["127.0.0.1", "::1"].iter().any(|host| {
        format!("{host}:{port}")
            .parse::<SocketAddr>()
            .or_else(|_| format!("[{host}]:{port}").parse::<SocketAddr>())
            .is_ok_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok())
    })
}

pub fn git_origin(directory: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub struct Worktree {
    pub directory: PathBuf,
    pub branch: Option<String>,
}

/// Every checked-out worktree of the repository that `directory` belongs to.
pub fn git_worktrees(directory: &Path) -> Vec<Worktree> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["worktree", "list", "--porcelain"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .split("\n\n")
        .filter_map(|block| {
            let mut directory = None;
            let mut branch = None;
            let mut usable = true;
            for line in block.lines() {
                if let Some(path) = line.strip_prefix("worktree ") {
                    directory = Some(PathBuf::from(path));
                } else if let Some(name) = line.strip_prefix("branch ") {
                    branch = Some(name.trim_start_matches("refs/heads/").to_owned());
                } else if line == "bare" || line.starts_with("prunable") {
                    usable = false;
                }
            }
            directory
                .filter(|path| usable && path.is_dir())
                .map(|directory| Worktree { directory, branch })
        })
        .collect()
}

/// A project name portman accepts: lowercase letters, digits and single dashes.
pub fn project_name(text: &str) -> String {
    let mut name = String::new();
    for character in text.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character);
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    name.trim_matches('-').chars().take(63).collect()
}

pub fn start_command<'a>(settings: &'a Settings, origin: Option<&str>) -> Option<&'a str> {
    settings
        .commands
        .iter()
        .find(|command| match (&command.repo, origin) {
            (None, _) => true,
            (Some(repo), Some(origin)) => origin.contains(repo.as_str()),
            (Some(_), None) => false,
        })
        .map(|command| command.run.as_str())
}
