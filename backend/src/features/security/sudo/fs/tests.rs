use super::*;

#[test]
fn shell_quotes_embedded_single_quotes() {
    assert_eq!(shell_quote("/var/log"), "'/var/log'");
    assert_eq!(shell_quote("/it's/a'file"), "'/it'\\''s/a'\\''file'");
    // Metacharacters stay inert inside the quotes.
    assert_eq!(shell_quote("$(rm -rf /); `id`"), "'$(rm -rf /); `id`'");
    assert_eq!(shell_quote("a'\nb"), "'a'\\''\nb'");
}

#[test]
fn parses_gnu_stat_lines() {
    // GNU stat -c '%F|%i|%s|%Y|%a|%U|%u|%G|%g' — 9 字段
    let info = parse_gnu_stat_line("regular file|12345|1024|1720000000|644|root|0|root|0").unwrap();
    assert_eq!(info.kind, "file");
    assert_eq!(info.size, 1024);
    assert_eq!(info.modified_at, 1720000000);
    assert_eq!(info.mode, "0644");
    assert_eq!(info.owner, "root");
    assert_eq!(info.group, "root");
    assert_eq!(info.owner_uid, Some(0));
    assert_eq!(info.group_gid, Some(0));

    // 普通用户 alice (uid=1000, gid=1000) 的 SUID 可执行文件
    let empty =
        parse_gnu_stat_line("regular empty file|99|0|1720000002|4755|alice|1000|wheel|1000")
            .unwrap();
    assert_eq!(empty.kind, "file");
    assert_eq!(empty.mode, "4755");
    assert_eq!(empty.owner_uid, Some(1000));
    assert_eq!(empty.group_gid, Some(1000));

    let dir = parse_gnu_stat_line("directory|2|4096|1720000001|755|root|0|root|0").unwrap();
    assert_eq!(dir.kind, "directory");
    assert_eq!(dir.mode, "0755");

    let link = parse_gnu_stat_line("symbolic link|98|11|1720000003|777|root|0|root|0").unwrap();
    assert_eq!(link.kind, "symlink");

    let device =
        parse_gnu_stat_line("character special file|97|0|1720000004|666|root|0|root|0").unwrap();
    assert_eq!(device.kind, "other");

    assert!(parse_gnu_stat_line("garbage").is_none());
    // 字段数不够（少 2 个 uid/gid）
    assert!(parse_gnu_stat_line("regular file|1|2|3|644|root|root").is_none());
    // mode 非法
    assert!(parse_gnu_stat_line("regular file|1|2|3|999|root|0|root|0").is_none());
}

#[test]
fn parses_bsd_stat_lines() {
    // BSD stat -f '%HT|%i|%z|%m|%Lp|%Su|%u|%Sg|%g' — 9 字段
    let dir = parse_bsd_stat_line("Directory|2|4096|1720000000|16877|root|0|wheel|0").unwrap();
    assert_eq!(dir.kind, "directory");
    assert_eq!(dir.mode, "0755"); // 16877 & 0o7777 = 0o755
    assert_eq!(dir.size, 4096);
    assert_eq!(dir.owner, "root");
    assert_eq!(dir.owner_uid, Some(0));
    assert_eq!(dir.group_gid, Some(0));

    let file = parse_bsd_stat_line("File|99|1024|1720000001|33188|alice|1000|staff|500").unwrap();
    assert_eq!(file.kind, "file");
    assert_eq!(file.mode, "0644"); // 33188 & 0o7777 = 0o644
    assert_eq!(file.group, "staff");
    assert_eq!(file.owner_uid, Some(1000));
    assert_eq!(file.group_gid, Some(500));

    let link = parse_bsd_stat_line("Symbolic Link|98|11|1720000002|41471|root|0|wheel|0").unwrap();
    assert_eq!(link.kind, "symlink");
    assert_eq!(link.mode, "0777"); // 41471 & 0o7777 = 0o7777
}

#[test]
fn parses_gnu_ls_output_with_epoch_times() {
    let output = "\
total 20
drwxr-xr-x  3 root root 4096 1720000000 .
drwxr-xr-x  1 root root 4096 1720000001 ..
-rw-r--r--  1 root root  123 1720000002 notes.txt
drwxr-xr-x  2 root root 4096 1720000003 sub dir with spaces
lrwxrwxrwx  1 root root   11 1720000004 link -> notes.txt
";
    let entries = parse_ls_output("/var/data", output);
    // Directories first, then case-insensitive names.
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["sub dir with spaces", "link", "notes.txt"]);

    let directory = &entries[0];
    assert_eq!(directory.kind, "directory");
    assert_eq!(directory.uri, "sftp:/var/data/sub dir with spaces");
    assert_eq!(directory.size, Some(4096));
    assert_eq!(directory.modified_at, Some(1720000003));
    assert_eq!(directory.permissions.as_deref(), Some("0755"));
    assert!(directory.content_type.is_none());
    assert_eq!(directory.owner.as_deref(), Some("root"));
    assert_eq!(directory.group.as_deref(), Some("root"));

    let link = &entries[1];
    assert_eq!(link.kind, "symlink");
    assert_eq!(link.uri, "sftp:/var/data/link");
    assert_eq!(link.size, Some(11));
    assert_eq!(link.permissions.as_deref(), Some("0777"));
    assert_eq!(link.owner.as_deref(), Some("root"));

    let file = &entries[2];
    assert_eq!(file.kind, "file");
    assert_eq!(file.size, Some(123));
    assert_eq!(file.permissions.as_deref(), Some("0644"));
    assert_eq!(file.content_type.as_deref(), Some("text/plain"));
    assert_eq!(file.owner.as_deref(), Some("root"));
    assert_eq!(file.group.as_deref(), Some("root"));
}

#[test]
fn parses_busybox_ls_output_with_classic_dates() {
    // BusyBox dates span three fields ("Jan 15 10:23" or "Jan 15  2024"),
    // so the mtime is unavailable while names still parse.
    let output = "\
drwxr-xr-x    3 root     root          4096 Jan 15 10:23 .
drwxr-xr-x    1 root     root          4096 Jan 15 10:23 ..
-rw-r--r--    1 root     root           123 Jan 15  2024 old.log
lrwxrwxrwx    1 root     root            11 Jan 15 10:23 lnk
";
    let entries = parse_ls_output("/mnt", output);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["lnk", "old.log"]);

    let log = &entries[1];
    assert_eq!(log.kind, "file");
    assert_eq!(log.size, Some(123));
    assert!(log.modified_at.is_none());
    assert_eq!(log.permissions.as_deref(), Some("0644"));
    assert_eq!(log.uri, "sftp:/mnt/old.log");
    assert_eq!(log.owner.as_deref(), Some("root"));
    assert_eq!(log.group.as_deref(), Some("root"));

    let link = &entries[0];
    assert_eq!(link.kind, "symlink");
    assert!(link.modified_at.is_none());
}

#[test]
fn converts_symbolic_modes_with_special_bits() {
    assert_eq!(mode_string_to_octal("drwxr-xr-x"), Some(0o755));
    assert_eq!(mode_string_to_octal("-rw-r--r--"), Some(0o644));
    assert_eq!(mode_string_to_octal("lrwxrwxrwx"), Some(0o777));
    // setuid + exec, setgid without exec, sticky + exec.
    assert_eq!(mode_string_to_octal("-rwsr-xr-x"), Some(0o4755));
    assert_eq!(mode_string_to_octal("-rwxr-Sr-x"), Some(0o2745));
    assert_eq!(mode_string_to_octal("drwxr-xr-t"), Some(0o1755));
    assert_eq!(mode_string_to_octal("drwsr-xr-t"), Some(0o5755));
    // ACL/selinux markers append an 11th character; first 10 still count.
    assert_eq!(mode_string_to_octal("drwxr-xr-x+"), Some(0o755));
    assert_eq!(mode_string_to_octal("short"), None);
}

#[test]
fn validates_octal_modes() {
    assert!(validate_octal_mode("755").is_ok());
    assert!(validate_octal_mode("0644").is_ok());
    assert!(validate_octal_mode("4755").is_ok());
    assert!(validate_octal_mode("1777").is_ok());
    assert!(validate_octal_mode("9").is_err());
    assert!(validate_octal_mode("999").is_err());
    assert!(validate_octal_mode("07777").is_err());
    assert!(validate_octal_mode("12").is_err());
    assert!(validate_octal_mode("").is_err());
    assert!(validate_octal_mode("rwx").is_err());
    assert!(validate_octal_mode("75 5").is_err());
}
