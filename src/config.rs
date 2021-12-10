use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use atomic::Atomic;
use figment::{Figment, Provider};
use itertools::Itertools;
use log::warn;
use parking_lot::RwLock;
use serde::Deserialize;
use tap::TapFallible;

use crate::errors::ConfigError;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub mode: Arc<Atomic<ApplyMode>>,
    pub interval: Arc<Atomic<Interval>>,
    pub walk: Arc<RwLock<WalkConfig>>,
}

#[inline]
const fn watcher_interval_default() -> Duration {
    Duration::from_secs(30)
}

#[inline]
const fn rescan_interval_default() -> Duration {
    Duration::from_secs(86400)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Deserialize)]
pub struct Interval {
    #[serde(with = "humantime_serde", default = "watcher_interval_default")]
    pub watch: Duration,
    #[serde(with = "humantime_serde", default = "rescan_interval_default")]
    pub rescan: Duration,
}

impl Default for Interval {
    fn default() -> Self {
        Self {
            watch: watcher_interval_default(),
            rescan: rescan_interval_default(),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyMode {
    DryRun,
    AddOnly,
    All,
}

impl Default for ApplyMode {
    fn default() -> Self {
        Self::AddOnly
    }
}

impl Config {
    /// Load config from provider.
    ///
    /// # Errors
    /// `Figment` if error occurs when collecting config.
    /// `Rule` if rule name is referenced but not defined.
    /// `NotADirectory` if there's directory given but not found.
    pub fn from(provider: impl Provider) -> Result<Self, ConfigError> {
        let pre_config = PreConfig::from(provider)?;
        Ok(Self {
            mode: Arc::new(Atomic::new(pre_config.mode)),
            interval: Arc::new(Atomic::new(pre_config.interval)),
            walk: Arc::new(RwLock::new(WalkConfig::from(
                pre_config.directories,
                &pre_config.rules,
                pre_config.skips,
            )?)),
        })
    }

    /// Reload config from provider.
    ///
    /// # Errors
    /// `Figment` if error occurs when collecting config.
    /// `Rule` if rule name is referenced but not defined.
    /// `NotADirectory` if there's directory given but not found.
    pub fn reload(&self, provider: impl Provider) -> Result<(), ConfigError> {
        let pre_config = PreConfig::from(provider)?;
        self.mode.store(pre_config.mode, Ordering::Relaxed);
        self.interval.store(pre_config.interval, Ordering::Relaxed);
        *self.walk.write() =
            WalkConfig::from(pre_config.directories, &pre_config.rules, pre_config.skips)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct WalkConfig {
    pub directories: Vec<Directory>,
    pub skips: HashSet<PathBuf>,
}

/// A `CoW` view of [`WalkConfig`](WalkConfig).
pub struct WalkConfigView<'a> {
    pub directories: Cow<'a, [Directory]>,
    pub skips: Cow<'a, HashSet<PathBuf>>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Directory {
    pub path: PathBuf,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Hash, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Rule {
    pub excludes: Vec<PathBuf>,
    #[serde(default)]
    pub if_exists: Vec<PathBuf>,
}

fn max_common_path(path_1: impl AsRef<Path>, path_2: impl AsRef<Path>) -> PathBuf {
    let max_common_path = path_1
        .as_ref()
        .components()
        .zip(path_2.as_ref().components())
        .try_fold(vec![], |mut prev, (l, r)| {
            if l == r {
                prev.push(l);
                ControlFlow::Continue(prev)
            } else {
                ControlFlow::Break(prev)
            }
        });
    match max_common_path {
        ControlFlow::Continue(components) | ControlFlow::Break(components) => {
            components.into_iter().collect()
        }
    }
}

impl WalkConfig {
    fn from(
        directories: Vec<PreDirectory>,
        rules: &HashMap<String, Rule>,
        skips: Vec<String>,
    ) -> Result<Self, ConfigError> {
        Ok(Self {
            directories: directories
                .into_iter()
                .map(|pre_directory| {
                    // start parsing rule names into rules
                    pre_directory
                        .rules
                        .into_iter()
                        .map(|rule_name| {
                            // try get rule by name
                            rules
                                .get(rule_name.as_str())
                                .cloned()
                                .ok_or(ConfigError::Rule(rule_name))
                        })
                        .try_collect()
                        .and_then(|rules| {
                            PathBuf::from(shellexpand::tilde(&pre_directory.path).as_ref())
                                .canonicalize()
                                .map_err(|_| ConfigError::NotFound(pre_directory.path.clone()))
                                .and_then(|path| {
                                    path.is_dir()
                                        .then(|| path.clone())
                                        .ok_or(ConfigError::NotADirectory(path))
                                })
                                .map(|path| (path, rules))
                        })
                        .map(|(path, rules)| {
                            // compose directory
                            Directory { path, rules }
                        })
                })
                .try_collect()?,
            skips: skips
                .into_iter()
                .filter_map(|path| {
                    PathBuf::from(shellexpand::tilde(&path).as_ref())
                        .canonicalize()
                        .tap_err(|e| {
                            warn!("Error when parsing skipped path {}: {}", path, e);
                        })
                        .ok()
                })
                .collect(),
        })
    }

    /// Get common root of all directories.
    #[must_use]
    pub fn root(&self) -> Option<PathBuf> {
        WalkConfigView::from(self).root()
    }
}

impl WalkConfigView<'_> {
    /// Get common root of all directories.
    #[must_use]
    pub fn root(&self) -> Option<PathBuf> {
        self.directories
            .iter()
            .map(|item| &item.path)
            .fold(None, |acc, x| {
                acc.map_or_else(|| Some(x.clone()), |acc| Some(max_common_path(acc, x)))
            })
    }

    #[must_use]
    pub fn into_owned(self) -> WalkConfig {
        WalkConfig {
            directories: self.directories.into_owned(),
            skips: self.skips.into_owned(),
        }
    }
}

impl From<WalkConfig> for WalkConfigView<'static> {
    fn from(c: WalkConfig) -> Self {
        Self {
            directories: c.directories.into(),
            skips: Cow::Owned(c.skips),
        }
    }
}

impl<'a> From<&'a WalkConfig> for WalkConfigView<'a> {
    fn from(c: &'a WalkConfig) -> Self {
        Self {
            directories: (&c.directories).into(),
            skips: Cow::Borrowed(&c.skips),
        }
    }
}

#[derive(Deserialize)]
struct PreConfig {
    #[serde(default)]
    mode: ApplyMode,
    #[serde(default)]
    interval: Interval,
    #[serde(default)]
    directories: Vec<PreDirectory>,
    #[serde(default)]
    skips: Vec<String>,
    #[serde(default)]
    rules: HashMap<String, Rule>,
}

#[derive(Deserialize)]
struct PreDirectory {
    path: String,
    rules: Vec<String>,
}

impl PreConfig {
    fn from(provider: impl Provider) -> Result<Self, figment::Error> {
        Figment::from(provider).extract()
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use figment::providers::{Format, Yaml};
    use maplit::hashset;

    use crate::config::{ApplyMode, Config, Directory, Interval, Rule, WalkConfig};
    use crate::errors::ConfigError;

    macro_rules! path {
        ($s: expr) => {
            PathBuf::from_str($s).unwrap()
        };
    }
    macro_rules! cwd_path {
        ($s: expr) => {
            std::env::current_dir().unwrap().join($s)
        };
    }

    #[test]
    fn must_parse_simple() {
        static SIMPLE: &str = include_str!("../tests/configs/simple.yaml");
        let config = Config::from(Yaml::string(SIMPLE)).expect("must parse config");

        let rule_a = Rule {
            excludes: vec![path!("exclude_a")],
            if_exists: vec![],
        };
        let rule_b = Rule {
            excludes: vec![path!("exclude_b")],
            if_exists: vec![],
        };
        let rule_d = Rule {
            excludes: vec![path!("exclude_d1"), path!("exclude_d2")],
            if_exists: vec![path!("a"), path!("b")],
        };

        assert_eq!(config.mode.load(Ordering::Relaxed), ApplyMode::DryRun);
        assert_eq!(
            config.interval.load(Ordering::Relaxed),
            Interval {
                watch: Duration::from_secs(60),
                rescan: Duration::from_secs(43200),
            }
        );
        assert_eq!(
            &*config.walk.read(),
            &WalkConfig {
                directories: vec![
                    Directory {
                        path: cwd_path!("tests/mock_dirs/path_a"),
                        rules: vec![rule_a, rule_b.clone()],
                    },
                    Directory {
                        path: cwd_path!("tests/mock_dirs/path_b"),
                        rules: vec![rule_b, rule_d],
                    },
                ],
                skips: hashset![cwd_path!("tests/mock_dirs/path_b")],
            }
        );
    }

    #[test]
    fn must_reload() {
        static SIMPLE: &str = include_str!("../tests/configs/simple.yaml");
        static SIMPLE_RELOADED: &str = include_str!("../tests/configs/simple_reload.yaml");
        let config = Config::from(Yaml::string(SIMPLE)).expect("must parse config");
        config
            .reload(Yaml::string(SIMPLE_RELOADED))
            .expect("must reload config");

        let rule_a = Rule {
            excludes: vec![path!("exclude_b")],
            if_exists: vec![],
        };
        let rule_b = Rule {
            excludes: vec![path!("exclude_c")],
            if_exists: vec![],
        };

        assert_eq!(config.mode.load(Ordering::Relaxed), ApplyMode::All);
        assert_eq!(
            config.interval.load(Ordering::Relaxed),
            Interval {
                watch: Duration::from_secs(120),
                rescan: Duration::from_secs(86400),
            }
        );
        assert_eq!(
            &*config.walk.read(),
            &WalkConfig {
                directories: vec![Directory {
                    path: cwd_path!("tests/mock_dirs/path_b"),
                    rules: vec![rule_a, rule_b],
                },],
                skips: hashset![cwd_path!("tests/mock_dirs/path_a")],
            }
        );
    }

    #[test]
    fn must_fail_broken_rule() {
        static BROKEN: &str = include_str!("../tests/configs/broken_rule.yaml");
        let provider = Yaml::string(BROKEN);
        assert_eq!(
            Config::from(provider).expect_err("must fail"),
            ConfigError::Rule(String::from("a"))
        );
    }

    #[test]
    fn must_fail_broken_dir() {
        static BROKEN: &str = include_str!("../tests/configs/broken_dir.yaml");
        let provider = Yaml::string(BROKEN);
        assert_eq!(
            Config::from(provider).expect_err("must fail"),
            ConfigError::NotADirectory(cwd_path!("tests/mock_dirs/some_file"))
        );
    }

    #[test]
    fn must_fail_missing_dir() {
        static BROKEN: &str = include_str!("../tests/configs/missing_dir.yaml");
        let provider = Yaml::string(BROKEN);
        assert_eq!(
            Config::from(provider).expect_err("must fail"),
            ConfigError::NotFound(String::from("tests/mock_dirs/non_exist"))
        );
    }

    #[test]
    fn must_allow_missing_skip_dir() {
        static BROKEN: &str = include_str!("../tests/configs/allow_missing_skip_dir.yaml");
        let provider = Yaml::string(BROKEN);
        assert_eq!(
            &*Config::from(provider)
                .expect("must parse config")
                .walk
                .read(),
            &WalkConfig {
                directories: vec![],
                skips: HashSet::new(),
            }
        );
    }
}
