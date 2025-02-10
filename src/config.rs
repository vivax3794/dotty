use std::collections::{HashMap, HashSet};
use std::io;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use fs_extra::dir::CopyOptions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
enum ShorthandOrTable<T> {
    String(Box<str>),
    Value(T),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(from = "ShorthandOrTable<T>", into = "ShorthandOrTable<T>")]
struct SupportsShorthand<T: From<Box<str>> + Clone>(T);

impl<T: From<Box<str>> + Clone> From<ShorthandOrTable<T>> for SupportsShorthand<T>
where
    T: From<Box<str>>,
{
    fn from(value: ShorthandOrTable<T>) -> Self {
        match value {
            ShorthandOrTable::String(x) => Self(x.into()),
            ShorthandOrTable::Value(x) => Self(x),
        }
    }
}

impl<T: From<Box<str>> + Clone> From<SupportsShorthand<T>> for ShorthandOrTable<T> {
    fn from(value: SupportsShorthand<T>) -> Self {
        ShorthandOrTable::Value(value.0)
    }
}

impl<T: From<Box<str>> + Clone> Deref for SupportsShorthand<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: From<Box<str>> + Clone> DerefMut for SupportsShorthand<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Config {
    managers: HashMap<Box<str>, Manager>,
    packages: HashMap<Box<str>, HashSet<Box<str>>>,
    module: Module,
    dotty: DottyConfig,
    hooks: Hooks,
    files: HashMap<Box<str>, SupportsShorthand<File>>,
    template: TemplateContext,
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
    once: HashMap<Box<str>, SupportsShorthand<Hook>>,
    update: HashMap<Box<str>, SupportsShorthand<Hook>>,
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
    pub seperator: Box<str>,
    pub priority: u8,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            add: None,
            remove: None,
            update: None,
            sudo: false,
            seperator: " ".into(),
            priority: 50,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
#[serde(transparent)]
struct TemplateContext(HashMap<Box<str>, TemplateValue>);

#[derive(Serialize, Deserialize, Debug, Eq)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
enum TemplateValue {
    Value(Box<str>),
    Mapping(HashMap<Box<str>, TemplateValue>),
    Sequence(Vec<TemplateValue>),
}

impl PartialEq for TemplateValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Value(a), Self::Value(b)) => a == b,
            (Self::Mapping(a), Self::Mapping(b)) => a == b,
            (Self::Sequence(a), Self::Sequence(b)) => a.iter().all(|e| b.contains(e)),
            _ => false,
        }
    }
}

impl TemplateValue {
    fn combine(&mut self, other: TemplateValue) -> Result<()> {
        match (self, other) {
            (Self::Value(b), Self::Value(a)) => {
                return Err(anyhow!("Duplicate value in tempalte {a} and {b}"))
            }
            (Self::Sequence(me), Self::Sequence(other)) => me.extend(other),
            (Self::Mapping(me), Self::Mapping(other)) => {
                for (key, value) in other {
                    if let Some(current) = me.get_mut(&key) {
                        current.combine(value).context(format!("in {key}"))?;
                    } else {
                        me.insert(key, value);
                    }
                }
            }
            (me, other) => {
                return Err(anyhow!("Incompatible template values {me:?} and {other:?}"))
            }
        }

        Ok(())
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
                    seperator: " ".into(),
                    priority: 50,
                },
            )]),
            module: Module::default(),
            packages: HashMap::from([(
                "pacman".into(),
                HashSet::from(["neovim".into(), "git".into()]),
            )]),
            hooks: Hooks::default(),
            dotty: DottyConfig {},
            files: HashMap::new(),
            template: TemplateContext::default(),
        }
    }

    pub fn combine(&mut self, other: Config) -> Result<()> {
        self.managers.extend(other.managers);
        self.hooks.once.extend(other.hooks.once);
        self.hooks.update.extend(other.hooks.update);
        self.files.extend(other.files);

        for (manager, packages) in other.packages {
            self.packages.entry(manager).or_default().extend(packages);
        }

        for (key, value) in other.template.0 {
            if let Some(current) = self.template.0.get_mut(&key) {
                current.combine(value)?;
            } else {
                self.template.0.insert(key, value);
            }
        }

        Ok(())
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
            self.combine(config)?;
        }
        self.module = Module::default();
        Ok(())
    }

    pub fn update(&self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        let empty = HashSet::new();
        for (name, manager) in self.managers.iter() {
            if let Some(command) = &manager.update {
                let command = if manager.sudo {
                    format!("sudo {}", command)
                } else {
                    command.to_string()
                };

                let packages = self.packages.get(name).unwrap_or(&empty);

                if !manager.seperator.is_empty() {
                    let joined = packages
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(&manager.seperator);
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
            changes.push(Change::RawCommand {
                command: hook.command.clone(),
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
        let empty = HashSet::new();
        for mananger in managers {
            let new_packages = self.packages.get(mananger).unwrap_or(&empty);
            let current_packages = old.packages.get(mananger).unwrap_or(&empty);

            let added = new_packages.difference(current_packages);
            let removed = current_packages.difference(new_packages);

            let added = added.map(|x| (*x).clone()).collect::<Vec<_>>();
            let removed = removed.map(|x| (*x).clone()).collect::<Vec<_>>();

            if !removed.is_empty() {
                changes.push(Change::RemovePackage {
                    manager: mananger.clone(),
                    packages: removed,
                });
            }
            if !added.is_empty() {
                changes.push(Change::AddPackage {
                    manager: mananger.clone(),
                    packages: added,
                });
            }
        }

        for (name, hook) in self.hooks.once.iter() {
            let run_hook = if let Some(old_value) = old.hooks.once.get(name) {
                hook.command != old_value.command
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

        let redo_all_templates = self.template != old.template;

        for (target, file) in self.files.iter() {
            let is_new = !old.files.contains_key(target);

            let source = shellexpand::tilde(&file.source);
            let target = shellexpand::tilde(target);

            let source = PathBuf::from_str(&source).unwrap();
            let target = PathBuf::from_str(&target).unwrap();

            let source = source.canonicalize().unwrap_or(source);
            let target = target.canonicalize().unwrap_or(target);

            let is_template = source.extension().is_some_and(|ext| ext == "tera");

            // TODO: Make directory handling smarter
            // TODO: Make template handling smarter
            if is_new || !target.exists() || source.is_dir() || (is_template && redo_all_templates)
            {
                changes.push(Change::CopyFile((**file).clone(), target));
            } else {
                let source_changed = std::fs::metadata(&source)?.modified()?;
                let target_changed = std::fs::metadata(&target)?.modified()?;

                if source_changed > target_changed {
                    changes.push(Change::CopyFile((**file).clone(), target));
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

                let is_template = source.extension().is_some_and(|ext| ext == "tera");

                if is_template {
                    if file.sudo {
                        return Err(anyhow!("Can not use `sudo` with templates"));
                    }

                    let mut templater = tera::Tera::default();
                    templater.add_template_file(source, Some("template"))?;
                    let context = tera::Context::from_serialize(&config.template)?;
                    let rendered = templater.render("template", &context)?;

                    actions.push(Action::StoreFile(rendered.into_boxed_str(), target));
                } else if file.sudo {
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
    if !manager.seperator.is_empty() {
        let args = packages.join(&manager.seperator);
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
    StoreFile(Box<str>, PathBuf),
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
            Self::StoreFile(_, target) => format!("<template> -> {}", target.display()).purple(),
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
                    std::fs::create_dir_all(parent)?;
                    std::fs::copy(&source, &target)?;
                }
            }
            Self::CopySudo(source, target) => {
                sudo_copy(&source, &target)?;
            }
            Self::StoreFile(content, target) => {
                let parent = target.parent().unwrap();
                std::fs::create_dir_all(parent)?;
                std::fs::write(target, content.as_ref())?;
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
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to create directory",
        ));
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
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to copy directory",
        ));
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
