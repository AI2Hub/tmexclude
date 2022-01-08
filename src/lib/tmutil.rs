//! Utils needed to operate on `TimeMachine`.
use std::borrow::Borrow;
use std::ops::{Add, AddAssign};
use std::path::{Path, PathBuf};
use std::ptr;

use core_foundation::base::{TCFType, ToVoid};
use core_foundation::number::{kCFBooleanFalse, kCFBooleanTrue};
use core_foundation::url;
use core_foundation::url::kCFURLIsExcludedFromBackupKey;
use log::{info, warn};
use tap::TapFallible;

pub fn is_excluded(path: impl AsRef<Path>) -> std::io::Result<bool> {
    let path = path.as_ref();
    Ok(
        xattr::get(path, "com.apple.metadata:com_apple_backup_excludeItem")
            .tap_err(|e| warn!("Error when querying xattr of file {:?}: {}", path, e))?
            .is_some(),
    )
}

#[derive(Debug, Clone, Default)]
pub struct ExclusionActionBatch {
    pub add: Vec<PathBuf>,
    pub remove: Vec<PathBuf>,
}

impl ExclusionActionBatch {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
    pub fn apply(self, remove: bool) {
        self.add.into_iter().for_each(|path| {
            info!("Excluding {:?} from backups", path);
            ExclusionAction::Add(path).apply();
        });
        if remove {
            self.remove.into_iter().for_each(|path| {
                info!("Including {:?} to backups", path);
                ExclusionAction::Remove(path).apply();
            });
        }
    }
}

impl<T> From<T> for ExclusionActionBatch
where
    T: Iterator<Item = ExclusionAction>,
{
    fn from(it: T) -> Self {
        let mut this = Self::default();
        it.for_each(|item| match item {
            ExclusionAction::Add(path) => this.add.push(path),
            ExclusionAction::Remove(path) => this.remove.push(path),
        });
        this
    }
}

impl<T: Borrow<Self>> Add<T> for ExclusionActionBatch {
    type Output = Self;

    fn add(mut self, rhs: T) -> Self::Output {
        self.add.extend_from_slice(&rhs.borrow().add);
        self.remove.extend_from_slice(&rhs.borrow().remove);
        self
    }
}

impl AddAssign for ExclusionActionBatch {
    fn add_assign(&mut self, rhs: Self) {
        self.add.extend_from_slice(&rhs.add);
        self.remove.extend_from_slice(&rhs.remove);
    }
}

#[derive(Debug, Clone)]
pub enum ExclusionAction {
    Add(PathBuf),
    Remove(PathBuf),
}

impl ExclusionAction {
    pub fn apply(self) {
        let value = unsafe {
            if matches!(self, Self::Add(_)) {
                kCFBooleanTrue
            } else {
                kCFBooleanFalse
            }
        };
        match self {
            Self::Add(path) | Self::Remove(path) => {
                if let Some(path) = url::CFURL::from_path(path, false) {
                    unsafe {
                        url::CFURLSetResourcePropertyForKey(
                            path.as_concrete_TypeRef(),
                            kCFURLIsExcludedFromBackupKey,
                            value.to_void(),
                            ptr::null_mut(),
                        );
                    }
                }
            }
        }
    }
}
