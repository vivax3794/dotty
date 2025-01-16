use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::io;
use std::process::Command;

use anyhow::Result;
use colored::Colorize;
use fs_extra::dir::CopyOptions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
enum StringyValue<T> {
    String(Box<str>),
    Value(T),
}

impl<T> StringyValue<T>
where
    T: From<Box<str>> + Clone,
{
    fn value(&self) -> T {
        match self {
            Self::String(x) => x.clone().into(),
            Self::Value(x) => x.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Config {
    managers: HashMap<Box<str>, Manager>,
    packages: HashMap<Box<str>, Vec<Box<str>>>,
    module: Module,
    dotty: DottyConfig,
    hooks: Hooks,
    files: HashMap<Box<str>, StringyValue<File>>,
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Module {
    import: HashSet<Box<str>>,
    disable: bool,
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    once: HashMap<Box<str>, StringyValue<Hook>>,
    update: HashMap<Box<str>, StringyValue<Hook>>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct File {
    source: Box<str>,
    priority: u8,
    post_hook: Option<Box<str>>,
    sudo: bool,
}

impl Default for File {
    fn default() -> Self {
        Self {
            source: "".into(),
            priority: 50,
            post_hook: None,
            sudo: false,
        }
    }
}

impl From<Box<str>> for File {
    fn from(value: Box<str>) -> Self {
        Self {
            source: value,
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub command: Box<str>,
    pub priority: u8,
}

impl From<Box<str>> for Hook {
    fn from(value: Box<str>) -> Self {
        Self {
            command: value,
            ..Default::default()
        }
    }
}

impl Default for Hook {
    fn default() -> Self {
        Self {
            command: "".into(),
            priority: 50,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct DottyConfig {}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Manager {
    pub add: Option<Box<str>>,
    pub remove: Option<Box<str>>,
    pub update: Option<Box<str>>,
    pub sudo: bool,
    pub seperator: Option<Box<str>>,
    pub priority: u8,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            add: None,
            remove: None,
            update: None,
            sudo: false,
            seperator: Some(" ".into()),
            priority: 50,
        }
    }
}

impl Config {
    pub fn example() -> Self {
        Self {
            managers: HashMap::from([(
                "pacman".into(),
                Manager {
                    add: Some("pacman -S #:?".into()),
                    remove: Some("pacman -Rns #:?".into()),
                    update: Some("pacman -Syu".into()),
                    sudo: true,
                    seperator: Some(" ".into()),
                    priority: 50,
                },
            )]),
            module: Module::default(),
            packages: HashMap::from([("pacman".into(), vec!["neovim".into(), "git".into()])]),
            hooks: Hooks::default(),
            dotty: DottyConfig {},
            files: HashMap::new(),
        }
    }

    pub fn combine(&mut self, other: Config) {
        self.managers.extend(other.managers);
        self.hooks.once.extend(other.hooks.once);
        self.hooks.update.extend(other.hooks.update);
        self.files.extend(other.files);

        for (manager, packages) in other.packages {
            self.packages.entry(manager).or_default().extend(packages);
        }
    }

    pub fn load_dependencies(&mut self, directory: &Path) -> Result<()> {
        if self.module.disable {
            *self = Self::default();
            return Ok(());
        }

        for module in self.module.import.clone().into_iter() {
            let path = directory.join(PathBuf::from_str(&module)?);
            let content = std::fs::read_to_string(&path)?;
            let mut config: Self = toml::from_str(&content)?;
            let new_directory = path.parent().unwrap_or(directory);
            config.load_dependencies(new_directory)?;
            self.combine(config);
        }
        self.module = Module::default();
        Ok(())
    }

    pub fn update(&self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        let empty = Vec::new();
        for (name, manager) in self.managers.iter() {
            if let Some(command) = &manager.update {
                let command = if manager.sudo {
                    format!("sudo {}", command)
                } else {
                    command.to_string()
                };

                let packages = self.packages.get(name).unwrap_or(&empty);

                if let Some(seperator) = &manager.seperator {
                    let joined = packages.join(seperator);
                    changes.push(Change::RawCommand {
                        command: command.replace("#:?", &joined).into(),
                        priority: manager.priority,
                    });
                } else {
                    for package in packages {
                        changes.push(Change::RawCommand {
                            command: command.replace("#:?", package).into(),
                            priority: manager.priority,
                        });
                    }
                }
            }
        }

        for hook in self.hooks.update.values() {
            let hook = hook.value();
            changes.push(Change::RawCommand {
                command: hook.command,
                priority: hook.priority,
            });
        }

        changes.sort_by_key(|x| x.priority(self));

        Ok(changes)
    }

    pub fn diff(&self, old: Config) -> Result<Vec<Change>> {
        let mut changes = Vec::new();

        let managers = self.managers.keys().collect::<Vec<_>>();

        // Since the hashmap returns references `unwrap_or` needs a reference for the default
        let empty = Vec::new();
        for mananger in managers {
            let new_packages = self.packages.get(mananger).unwrap_or(&empty);
            let current_packages = old.packages.get(mananger).unwrap_or(&empty);

            let new_packages: HashSet<&Box<str>> = HashSet::from_iter(new_packages.iter());
            let current_packages = HashSet::from_iter(current_packages.iter());

            let added = new_packages.difference(&current_packages);
            let removed = current_packages.difference(&new_packages);

            let added = added.map(|x| (*x).clone()).collect::<Vec<_>>();
            let removed = removed.map(|x| (*x).clone()).collect::<Vec<_>>();

            if !added.is_empty() {
                changes.push(Change::AddPackage {
                    manager: mananger.clone(),
                    packages: added,
                });
            }
            if !removed.is_empty() {
                changes.push(Change::RemovePackage {
                    manager: mananger.clone(),
                    packages: removed,
                });
            }
        }

        for (name, hook) in self.hooks.once.iter() {
            let hook = hook.value();
            let run_hook = if let Some(old_value) = old.hooks.once.get(name) {
                hook.command != old_value.value().command
            } else {
                true
            };
            if run_hook {
                changes.push(Change::RawCommand {
                    command: hook.command.clone(),
                    priority: hook.priority,
                });
            }
        }

        for (target, file) in self.files.iter() {
            let file = file.value();
            let is_new = !old.files.contains_key(target);

            let source = shellexpand::tilde(&file.source);
            let target = shellexpand::tilde(target);

            let source = PathBuf::from_str(&source).unwrap();
            let target = PathBuf::from_str(&target).unwrap();

            let source = source.canonicalize().unwrap_or(source);
            let target = target.canonicalize().unwrap_or(target);

            if is_new || !target.exists() || source.is_dir() {
                changes.push(Change::CopyFile(file.clone(), target));
            } else {
                let source_changed = std::fs::metadata(&source)?.modified()?;
                let target_changed = std::fs::metadata(&target)?.modified()?;

                if source_changed > target_changed {
                    changes.push(Change::CopyFile(file.clone(), target));
                }
            }
        }

        changes.sort_by_key(|x| x.priority(self));

        Ok(changes)
    }
}

#[derive(Debug)]
pub enum Change {
    AddPackage {
        manager: Box<str>,
        packages: Vec<Box<str>>,
    },
    RemovePackage {
        manager: Box<str>,
        packages: Vec<Box<str>>,
    },
    CopyFile(File, PathBuf),
    RawCommand {
        command: Box<str>,
        priority: u8,
    },
}

impl Change {
    pub fn priority(&self, config: &Config) -> u8 {
        match self {
            Self::AddPackage { manager, .. } | Self::RemovePackage { manager, .. } => {
                let manager = config.managers.get(manager).unwrap();
                manager.priority
            }
            Self::RawCommand { priority, .. } | Self::CopyFile(File { priority, .. }, _) => {
                *priority
            }
        }
    }

    pub fn render(&self) -> colored::ColoredString {
        match self {
            Self::AddPackage {
                manager, packages, ..
            } => {
                let joined = packages.join(", ");
                format!("{}: {}", manager, joined).green()
            }
            Self::RemovePackage {
                manager, packages, ..
            } => {
                let joined = packages.join(", ");
                format!("{}: {}", manager, joined).red()
            }
            Self::CopyFile(file, target) => {
                format!("{} -> {}", file.source, target.display()).purple()
            }
            Self::RawCommand { command, .. } => format!("{}", command).cyan(),
        }
    }

    pub fn action(self, config: &Config) -> Result<Vec<Action>> {
        match self {
            Self::AddPackage { manager, packages } => {
                let manager = config
                    .managers
                    .get(&manager)
                    .ok_or(anyhow::anyhow!("Manager {} not found", manager))?;

                if let Some(command) = &manager.add {
                    construct_command(packages, manager, command)
                } else {
                    Ok(vec![])
                }
            }
            Self::RemovePackage { manager, packages } => {
                let manager = config
                    .managers
                    .get(&manager)
                    .ok_or(anyhow::anyhow!("Manager {} not found", manager))?;

                if let Some(command) = &manager.remove {
                    construct_command(packages, manager, command)
                } else {
                    Ok(vec![])
                }
            }
            Self::RawCommand { command, .. } => Ok(vec![Action::Run {
                command,
                sudo: false,
            }]),
            Self::CopyFile(file, target) => {
                let mut actions = Vec::with_capacity(2);
                let source = PathBuf::from_str(&file.source).unwrap();

                if file.sudo {
                    actions.push(Action::CopySudo(source, target));
                } else {
                    actions.push(Action::Copy(source, target));
                }

                if let Some(command) = &file.post_hook {
                    actions.push(Action::Run {
                        command: command.clone(),
                        sudo: false,
                    })
                }
                Ok(actions)
            }
        }
    }
}

fn construct_command(
    packages: Vec<Box<str>>,
    manager: &Manager,
    command: &str,
) -> std::result::Result<Vec<Action>, anyhow::Error> {
    if let Some(seperator) = &manager.seperator {
        let args = packages.join(seperator);
        Ok(vec![Action::Run {
            command: command.replace("#:?", &args).into(),
            sudo: manager.sudo,
        }])
    } else {
        Ok(packages
            .into_iter()
            .map(|x| Action::Run {
                command: command.replace("#:?", &x).into(),
                sudo: manager.sudo,
            })
            .collect())
    }
}

#[derive(Debug)]
pub enum Action {
    Run { command: Box<str>, sudo: bool },
    Copy(PathBuf, PathBuf),
    CopySudo(PathBuf, PathBuf),
}

impl Action {
    pub fn render(&self) -> colored::ColoredString {
        match self {
            Self::Run {
                command,
                sudo: false,
            } => format!("{}", command).yellow(),
            Self::Run {
                command,
                sudo: true,
            } => format!("sudo {}", command).yellow(),
            Self::Copy(source, target) | Self::CopySudo(source, target) => {
                format!("{} -> {}", source.display(), target.display()).purple()
            }
        }
    }

    pub fn execute(self) -> Result<()> {
        match self {
            Self::Run { command, sudo } => {
                let command = if sudo {
                    format!("sudo {}", command)
                } else {
                    command.into()
                };

                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .status()?
                    .exit_ok()?;
            }
            Self::Copy(source, target) => {
                if source.is_dir() {
                    std::fs::create_dir_all(&target)?;
                    fs_extra::dir::copy(
                        &source,
                        &target,
                        &CopyOptions::new().overwrite(true).content_only(true),
                    )?;
                } else {
                    let parent = target.parent().unwrap();
                    std::fs::create_dir_all(&parent)?;
                    std::fs::copy(&source, &target)?;
                }
            }
            Self::CopySudo(source, target) => {
                sudo_copy(&source, &target)?;
            }
        }

        Ok(())
    }
}

fn sudo_create_dir_all(path: &Path) -> io::Result<()> {
    let path_str = path.to_str().unwrap();
    let status = Command::new("sudo")
        .arg("mkdir")
        .arg("-p")
        .arg(path_str)
        .status()?;

    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "Failed to create directory"));
    }

    Ok(())
}

fn sudo_copy_file(source: &Path, target: &Path) -> io::Result<()> {
    let source_str = source.to_str().unwrap();
    let target_str = target.to_str().unwrap();
    let status = Command::new("sudo")
        .arg("cp")
        .arg(source_str)
        .arg(target_str)
        .status()?;

    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "Failed to copy file"));
    }

    Ok(())
}

fn sudo_copy_dir(source: &Path, target: &Path) -> io::Result<()> {
    let source_str = source.to_str().unwrap();
    let target_str = target.to_str().unwrap();

    // Construct `fs_extra`-style copy command manually
    let mut cmd = Command::new("sudo");
    cmd.arg("cp")
        .arg("-r")
        .arg(source_str)
        .arg(target_str)
        .arg("--remove-destination"); // Force overwrite of existing files

    let status = cmd.status()?;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "Failed to copy directory"));
    }

    Ok(())
}

fn sudo_copy(source: &Path, target: &Path) -> io::Result<()> {
    if source.is_dir() {
        // Handle directory copy
        sudo_create_dir_all(target)?;
        sudo_copy_dir(source, target)?;
    } else {
        // Handle file copy
        let parent = target.parent().unwrap();
        sudo_create_dir_all(parent)?;
        sudo_copy_file(source, target)?;
    }
    Ok(())
}
