//! An explicit data directory permits real desktop diagnostics without touching
//! the user's regular database, browser cache, or pending conversations.
use std::path::PathBuf;

pub fn directory(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut found = None;
    for (index, arg) in args.iter().enumerate() {
        if arg != "--user-data-dir" { continue; }
        if found.is_some() { return Err("--user-data-dir 只能指定一次".into()); }
        let path = args.get(index + 1).filter(|value| !value.starts_with("--"))
            .map(PathBuf::from).ok_or("--user-data-dir 需要绝对路径")?;
        if !path.is_absolute() { return Err("--user-data-dir 需要绝对路径".into()); }
        found = Some(path);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> { values.iter().map(|s| (*s).into()).collect() }
    #[test]
    fn regular_startup_has_no_override() {
        assert_eq!(directory(&args(&["supercode"])).unwrap(), None);
    }
    #[test]
    fn rejects_missing_relative_or_duplicate_directory() {
        for values in [vec!["supercode", "--user-data-dir"],
            vec!["supercode", "--user-data-dir", "relative"],
            vec!["supercode", "--user-data-dir", "--smoke-test"]] {
            assert!(directory(&args(&values)).is_err());
        }
        let path = std::env::temp_dir().join("SuperCode 测试");
        let path = path.to_str().unwrap();
        assert!(directory(&args(&["supercode", "--user-data-dir", path, "--user-data-dir", path])).is_err());
    }
    #[test]
    fn preserves_absolute_unicode_directory() {
        let path = std::env::temp_dir().join("SuperCode 测试");
        assert_eq!(directory(&args(&["supercode", "--user-data-dir", path.to_str().unwrap()])).unwrap(), Some(path));
    }
}
