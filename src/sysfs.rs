use std::path::Path;

pub fn read_sysfs_opt(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
