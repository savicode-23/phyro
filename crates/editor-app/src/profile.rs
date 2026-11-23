use std::path::{Path, PathBuf};

/// Choose per component. Older and newer editors may be running with live
/// locks on the same preferences, drafts, or crash session markers.
pub(crate) fn data_path(base: &Path, brand: &str, component: &str) -> PathBuf {
    let current = base.join(brand).join(component);
    let old_brand = if brand == "phyro" {
        "loopline"
    } else {
        "Loopline"
    };
    let legacy = base.join(old_brand).join(component);
    if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fresh_and_existing_components_keep_the_same_location_on_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("Loopline")).unwrap();
        let old_file = root.join("Loopline/preferences.json");
        fs::write(&old_file, "{}").unwrap();
        assert_eq!(data_path(root, "Phyro", "preferences.json"), old_file);
        let recovery = data_path(root, "Phyro", "Recovery");
        assert_eq!(recovery, root.join("Phyro/Recovery"));
        fs::create_dir_all(&recovery).unwrap();
        assert_eq!(data_path(root, "Phyro", "Recovery"), recovery);
        assert_eq!(data_path(root, "Phyro", "preferences.json"), old_file);
        let current_file = root.join("Phyro/preferences.json");
        fs::write(&current_file, "{}").unwrap();
        assert_eq!(data_path(root, "Phyro", "preferences.json"), current_file);
    }
}
