//! `docker exec … ls`/`stat` parsing and command construction.

use super::*;

/// Builds the directory listing command for both planes: the SSH shell
/// command (endpoint export + shell-quoted path) and the local argv (direct
/// exec, no shell). The container CLI is always included so `cli: "podman"`
/// keeps working.
pub fn list_command(
    engine: &EngineConfig,
    container_id: &str,
    path: &str,
) -> Result<(String, Vec<String>), String> {
    validate_container_id(container_id)?;
    let path = normalize_container_path(path)?;
    let ssh = format!(
        "{}{} exec {} ls -1ap -- {}",
        engine.env_prefix(),
        engine.cli,
        container_id,
        exec::shell_quote(&path)
    );
    let argv = vec![
        "exec".to_string(),
        container_id.to_string(),
        "ls".to_string(),
        "-1ap".to_string(),
        "--".to_string(),
        path,
    ];
    Ok((ssh, argv))
}

/// Builds the `stat -c '%F|%s|%Y|%a'` command for both planes.
pub fn stat_command(
    engine: &EngineConfig,
    container_id: &str,
    path: &str,
) -> Result<(String, Vec<String>), String> {
    validate_container_id(container_id)?;
    let path = normalize_container_path(path)?;
    let ssh = format!(
        "{}{} exec {} stat -c '%F|%s|%Y|%a' -- {}",
        engine.env_prefix(),
        engine.cli,
        container_id,
        exec::shell_quote(&path)
    );
    let argv = vec![
        "exec".to_string(),
        container_id.to_string(),
        "stat".to_string(),
        "-c".to_string(),
        "%F|%s|%Y|%a".to_string(),
        "--".to_string(),
        path,
    ];
    Ok((ssh, argv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ls_rows() {
        let output = "bin/\netc/\n.ssh\nserver.log\r\n..\n.\n";
        assert_eq!(
            parse_ls_output(output),
            vec![
                entry("bin", "dir"),
                entry("etc", "dir"),
                entry(".ssh", "file"),
                entry("server.log", "file"),
            ]
        );
    }

    #[test]
    fn parses_ls_rows_into_full_paths() {
        let entries = list_entries("/srv", "app/\nreport.log\n");
        assert_eq!(entries[0].path, "/srv/app");
        assert_eq!(entries[1].path, "/srv/report.log");
    }

    #[test]
    fn list_command_renders_both_planes() {
        let engine = test_engine("podman", None);
        let (ssh, argv) = list_command(&engine, "d4a7c9f1e2b3", "/var//log/").unwrap();
        assert_eq!(ssh, "podman exec d4a7c9f1e2b3 ls -1ap -- '/var/log'");
        assert_eq!(
            argv,
            vec!["exec", "d4a7c9f1e2b3", "ls", "-1ap", "--", "/var/log"]
        );
    }

    #[test]
    fn stat_command_quotes_the_path() {
        let engine = test_engine("docker", Some("unix:///tmp/d.sock"));
        let (ssh, argv) = stat_command(&engine, "d4a7c9f1e2b3", "/tmp/a b").unwrap();
        assert_eq!(
            ssh,
            "export DOCKER_HOST='unix:///tmp/d.sock' CONTAINER_HOST='unix:///tmp/d.sock'; \
             docker exec d4a7c9f1e2b3 stat -c '%F|%s|%Y|%a' -- '/tmp/a b'"
        );
        assert_eq!(argv[6], "/tmp/a b");
    }

    #[test]
    fn commands_reject_bad_paths_and_ids() {
        let engine = test_engine("docker", None);
        assert!(list_command(&engine, "not-hex", "/").is_err());
        assert!(list_command(&engine, "d4a7c9f1e2b3", "relative").is_err());
    }

    fn entry(name: &str, kind: &str) -> ContainerEntry {
        ContainerEntry {
            name: name.to_string(),
            path: name.to_string(),
            kind: kind.to_string(),
        }
    }
}
