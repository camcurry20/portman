mod model;

use model::{Project, Settings};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

const REFRESH: Duration = Duration::from_secs(2);
const SCAN: Duration = Duration::from_secs(10);
const TOOL_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// Everything the menu shows; the menu is rebuilt only when this changes.
#[derive(PartialEq, Eq)]
struct Snapshot {
    settings: Settings,
    projects: BTreeMap<String, Project>,
    listening: BTreeMap<String, bool>,
    branches: BTreeMap<String, String>,
    managed: Vec<String>,
}

struct App {
    tray: Option<TrayIcon>,
    snapshot: Option<Snapshot>,
    children: HashMap<String, Child>,
    origins: HashMap<PathBuf, Option<String>>,
    branches: BTreeMap<String, String>,
    scanned: Option<Instant>,
}

fn portman(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("/opt/homebrew/bin/portman");
    command
        .args(args)
        .env("PATH", TOOL_PATH)
        .env("HOMEBREW_PREFIX", "/opt/homebrew");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().map_err(|err| err.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn alert(message: &str) {
    rfd::MessageDialog::new()
        .set_title("Portman")
        .set_description(message)
        .set_level(rfd::MessageLevel::Warning)
        .show();
}

fn report(result: Result<String, String>) {
    if let Err(message) = result {
        alert(&message);
    }
}

impl App {
    fn origin(&mut self, directory: &Path) -> Option<String> {
        self.origins
            .entry(directory.to_owned())
            .or_insert_with(|| model::git_origin(directory))
            .clone()
    }

    fn reap(&mut self) {
        self.children
            .retain(|_, child| matches!(child.try_wait(), Ok(None)));
    }

    /// Registers every worktree of the known repositories, drops projects whose
    /// folder is gone, and records each project's branch.
    fn scan(&mut self) {
        let mut projects = model::load_projects();
        if projects
            .values()
            .any(|project| project.directory.as_ref().is_some_and(|dir| !dir.is_dir()))
        {
            let _ = portman(&["cleanup"], None);
            projects = model::load_projects();
        }

        let mut known: HashMap<PathBuf, String> = projects
            .iter()
            .filter_map(|(name, project)| Some((project.directory.clone()?, name.clone())))
            .collect();
        let mut branches = BTreeMap::new();
        let mut visited = Vec::new();
        for directory in known.keys().cloned().collect::<Vec<_>>() {
            if visited.contains(&directory) {
                continue;
            }
            let worktrees = model::git_worktrees(&directory);
            let repo = worktrees
                .first()
                .and_then(|main| main.directory.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            for worktree in worktrees {
                visited.push(worktree.directory.clone());
                if !known.contains_key(&worktree.directory) {
                    let folder = worktree
                        .directory
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let base = model::project_name(&format!("{repo}-{folder}"));
                    let name = (1..50)
                        .map(|n| {
                            if n == 1 {
                                base.clone()
                            } else {
                                format!("{base}-{n}")
                            }
                        })
                        .find(|candidate| !projects.contains_key(candidate));
                    if let Some(name) = name
                        && portman(&["create", &name, "--no-link"], Some(&worktree.directory))
                            .is_ok()
                    {
                        projects.insert(
                            name.clone(),
                            Project {
                                port: 0,
                                directory: None,
                                linked_port: None,
                            },
                        );
                        known.insert(worktree.directory.clone(), name);
                    }
                }
                if let (Some(name), Some(branch)) =
                    (known.get(&worktree.directory), worktree.branch)
                {
                    branches.insert(name.clone(), branch);
                }
            }
        }
        self.branches = branches;
        self.scanned = Some(Instant::now());
    }

    fn snapshot(&mut self, force_scan: bool) -> Snapshot {
        self.reap();
        if force_scan || self.scanned.is_none_or(|at| at.elapsed() >= SCAN) {
            self.scan();
        }
        let projects = model::load_projects();
        let listening = projects
            .iter()
            .map(|(name, project)| (name.clone(), model::is_listening(project.port)))
            .collect();
        let mut managed: Vec<String> = self.children.keys().cloned().collect();
        managed.sort();
        Snapshot {
            settings: model::load_settings(),
            projects,
            listening,
            branches: self.branches.clone(),
            managed,
        }
    }

    fn build_menu(&mut self, snapshot: &Snapshot) -> Menu {
        let menu = Menu::new();
        let separator = || PredefinedMenuItem::separator();

        for slot in &snapshot.settings.slots {
            let linked = snapshot
                .projects
                .iter()
                .find(|(_, project)| project.linked_port == Some(*slot))
                .map(|(name, _)| name.as_str());
            let describe = |name: &str| match snapshot.branches.get(name) {
                Some(branch) => format!("{name}  ({branch})"),
                None => name.to_owned(),
            };
            let label = match linked {
                Some(name) => format!("localhost:{slot}  →  {}", describe(name)),
                None => format!("localhost:{slot}  →  Not Linked"),
            };
            let submenu = Submenu::new(label, true);
            for name in snapshot.projects.keys() {
                let _ = submenu.append(&CheckMenuItem::with_id(
                    format!("link|{slot}|{name}"),
                    describe(name),
                    true,
                    linked == Some(name.as_str()),
                    None,
                ));
            }
            let _ = submenu.append(&separator());
            let _ = submenu.append(&MenuItem::with_id(
                format!("open|{slot}"),
                "Open In Browser",
                linked.is_some(),
                None,
            ));
            let _ = submenu.append(&MenuItem::with_id(
                format!("unlink|{slot}"),
                "Unlink",
                linked.is_some(),
                None,
            ));
            let _ = menu.append(&submenu);
        }

        let _ = menu.append(&separator());

        if snapshot.projects.is_empty() {
            let _ = menu.append(&MenuItem::new("No Projects Yet", false, None));
        }
        for (name, project) in &snapshot.projects {
            let running = snapshot.listening.get(name).copied().unwrap_or(false);
            let managed = snapshot.managed.contains(name);
            let status = if running { "●" } else { "○" };
            let branch = snapshot
                .branches
                .get(name)
                .map(|branch| format!("  ({branch})"))
                .unwrap_or_default();
            let submenu = Submenu::new(
                format!("{status}  {name}{branch}   :{}", project.port),
                true,
            );

            let origin = project
                .directory
                .clone()
                .and_then(|directory| self.origin(&directory));
            let has_command = model::start_command(&snapshot.settings, origin.as_deref()).is_some();

            if managed {
                let _ = submenu.append(&MenuItem::with_id(
                    format!("stop|{name}"),
                    "Stop Dev Server",
                    true,
                    None,
                ));
            } else if running {
                let _ = submenu.append(&MenuItem::new("Running Outside Portman", false, None));
            } else {
                let label = if has_command {
                    "Start Dev Server"
                } else {
                    "Start Dev Server (No Command Set)"
                };
                let _ = submenu.append(&MenuItem::with_id(
                    format!("start|{name}"),
                    label,
                    has_command && project.directory.is_some(),
                    None,
                ));
            }
            let _ = submenu.append(&MenuItem::with_id(
                format!("direct|{name}"),
                "Open In Browser",
                running,
                None,
            ));
            let _ = submenu.append(&MenuItem::with_id(
                format!("log|{name}"),
                "View Log",
                model::log_path(name).exists(),
                None,
            ));
            let _ = submenu.append(&MenuItem::with_id(
                format!("reveal|{name}"),
                "Show In Finder",
                project.directory.is_some(),
                None,
            ));
            let _ = submenu.append(&separator());
            let _ = submenu.append(&MenuItem::with_id(
                format!("remove|{name}"),
                "Remove Project",
                true,
                None,
            ));
            let _ = menu.append(&submenu);
        }

        let _ = menu.append(&separator());
        let _ = menu.append(&MenuItem::with_id("add", "Add Project Folder…", true, None));
        let _ = menu.append(&MenuItem::with_id(
            "cleanup",
            "Remove Projects With Missing Folders",
            true,
            None,
        ));
        let _ = menu.append(&MenuItem::with_id("settings", "Edit Settings…", true, None));
        let _ = menu.append(&separator());
        let _ = menu.append(&MenuItem::with_id("quit", "Quit Portman", true, None));
        menu
    }

    fn refresh(&mut self, force: bool) {
        let snapshot = self.snapshot(force);
        if !force && self.snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        let menu = self.build_menu(&snapshot);
        if let Some(tray) = &self.tray {
            tray.set_menu(Some(Box::new(menu)));
        } else {
            self.tray = TrayIconBuilder::new()
                .with_title("⇄")
                .with_tooltip("Portman")
                .with_menu(Box::new(menu))
                .build()
                .ok();
        }
        self.snapshot = Some(snapshot);
    }

    fn start(&mut self, name: &str) {
        let projects = model::load_projects();
        let Some(project) = projects.get(name) else {
            return;
        };
        let Some(directory) = project.directory.clone() else {
            return;
        };
        let settings = model::load_settings();
        let origin = self.origin(&directory);
        let Some(run) = model::start_command(&settings, origin.as_deref()) else {
            return;
        };
        let log = model::log_path(name);
        if let Some(parent) = log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(out) = std::fs::File::create(&log) else {
            alert("Could not create the log file.");
            return;
        };
        let Ok(err) = out.try_clone() else {
            return;
        };
        let spawned = Command::new("/bin/zsh")
            .args(["-lic", run])
            .current_dir(&directory)
            .env("PORT", project.port.to_string())
            .env("PORTMAN_PROJECT", name)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn();
        match spawned {
            Ok(child) => {
                self.children.insert(name.to_owned(), child);
            }
            Err(error) => alert(&format!("Could not start {name}: {error}")),
        }
    }

    fn stop(&mut self, name: &str) {
        if let Some(mut child) = self.children.remove(name) {
            terminate(&child);
            let _ = child.try_wait();
        }
    }

    fn stop_all(&mut self) {
        for (_, child) in self.children.drain() {
            terminate(&child);
        }
    }

    fn add_project(&mut self) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Choose A Project Folder")
            .pick_folder()
        else {
            return;
        };
        report(portman(&["create", "--no-link"], Some(&folder)));
    }

    fn handle(&mut self, id: &str) -> bool {
        let parts: Vec<&str> = id.splitn(3, '|').collect();
        let projects = model::load_projects();
        match parts.as_slice() {
            ["quit"] => {
                self.stop_all();
                return false;
            }
            ["add"] => self.add_project(),
            ["cleanup"] => report(portman(&["cleanup"], None)),
            ["settings"] => {
                model::load_settings();
                let _ = Command::new("/usr/bin/open")
                    .arg("-t")
                    .arg(model::settings_path())
                    .status();
            }
            ["link", slot, name] => {
                report(portman(&["link", slot, name], None));
            }
            ["unlink", slot] => report(portman(&["unlink", slot], None)),
            ["open", slot] => {
                let _ = open::that(format!("http://localhost:{slot}"));
            }
            ["start", name] => self.start(name),
            ["stop", name] => self.stop(name),
            ["direct", name] => {
                if let Some(project) = projects.get(*name) {
                    let _ = open::that(format!("http://localhost:{}", project.port));
                }
            }
            ["log", name] => {
                let _ = Command::new("/usr/bin/open")
                    .arg("-a")
                    .arg("Console")
                    .arg(model::log_path(name))
                    .status();
            }
            ["reveal", name] => {
                if let Some(directory) = projects.get(*name).and_then(|p| p.directory.as_ref()) {
                    let _ = open::that(directory);
                }
            }
            ["remove", name] => {
                self.stop(name);
                report(portman(&["delete", name], None));
            }
            _ => {}
        }
        true
    }
}

fn terminate(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: signals the process group this app created for the child.
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
    }
}

fn main() {
    let mut event_loop = EventLoopBuilder::<MenuEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(event);
    }));

    let mut app = App {
        tray: None,
        snapshot: None,
        children: HashMap::new(),
        origins: HashMap::new(),
        branches: BTreeMap::new(),
        scanned: None,
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + REFRESH);
        match event {
            Event::NewEvents(StartCause::Init) => app.refresh(true),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => app.refresh(false),
            Event::UserEvent(menu_event) => {
                if app.handle(menu_event.id.as_ref()) {
                    app.refresh(true);
                } else {
                    app.tray.take();
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
}
