//! MCP safety layer.

/// Depth cap for nested substitution scanning so pathological input cannot
/// recurse unboundedly (`$( $( $( …`.
pub(crate) const MAX_ASSESS_DEPTH: u8 = 4;

/// Byte ceiling for one `ssh_terminal_input` payload after normalization.
pub const MAX_TERMINAL_INPUT_BYTES: usize = 8 * 1024;

/// Verbs that are read-only in every argument shape.
pub(crate) const READ_ONLY_VERBS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "df",
    "du",
    "ps",
    "free",
    "uptime",
    "whoami",
    "id",
    "uname",
    "hostname",
    "w",
    "who",
    "last",
    "lastlog",
    "stat",
    "wc",
    "file",
    "cksum",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "echo",
    "printf",
    "date",
    "printenv",
    "which",
    "whereis",
    "type",
    "lsof",
    "ss",
    "netstat",
    "ping",
    "ping6",
    "traceroute",
    "tracepath",
    "nslookup",
    "dig",
    "host",
    "vmstat",
    "iostat",
    "sar",
    "mpstat",
    "dmesg",
    "lsblk",
    "lsmod",
    "lspci",
    "lsusb",
    "lsns",
    "lscpu",
    "nproc",
    "getent",
    "groups",
    "true",
    "false",
    "test",
    "[",
    "sleep",
    "history",
    "arch",
    "cut",
    "sort",
    "uniq",
    "tr",
    "column",
    "nl",
    "tac",
    "rev",
    "seq",
    "dirname",
    "basename",
    "readlink",
    "realpath",
    "pwd",
];

/// Verbs whose read-only-ness depends on the first subcommand.
pub(crate) const SUBCOMMAND_VERBS: &[(&str, &[&str])] = &[
    (
        "systemctl",
        &[
            "status",
            "list-units",
            "list-unit-files",
            "list-timers",
            "list-sockets",
            "list-dependencies",
            "list-jobs",
            "is-active",
            "is-enabled",
            "is-failed",
            "show",
            "cat",
            "help",
            "get-default",
        ],
    ),
    (
        "ip",
        &[
            "addr", "a", "address", "l", "link", "route", "r", "rule", "neigh", "n",
        ],
    ),
];

/// Top-level `docker <sub>` forms that only inspect (no object command).
pub(crate) const DOCKER_READ_SUBCOMMANDS: &[&str] = &[
    "ps", "images", "stats", "version", "info", "logs", "inspect", "top", "port", "events",
    "search",
];

/// `docker <object> <sub>` inspection forms. Mutating object commands
/// (create/rm/update/restart/deploy/prune/…) stay out on purpose.
pub(crate) const DOCKER_READ_OBJECTS: &[(&str, &[&str])] = &[
    (
        "container",
        &["ls", "ps", "inspect", "logs", "top", "port", "stats"],
    ),
    ("image", &["ls", "images", "inspect", "history"]),
    ("network", &["ls", "inspect"]),
    ("node", &["ls", "ps", "inspect", "top"]),
    ("service", &["ls", "ps", "inspect", "logs", "top"]),
    ("stack", &["ls", "ps", "services", "config"]),
    ("volume", &["ls", "inspect"]),
    ("system", &["df", "info", "events"]),
    ("compose", &["ps", "logs", "config", "ls"]),
    ("context", &["ls", "show"]),
];

/// Top-level `kubectl <sub>` forms that only inspect.
pub(crate) const KUBECTL_READ_SUBCOMMANDS: &[&str] = &[
    "get",
    "describe",
    "top",
    "logs",
    "version",
    "explain",
    "api-resources",
    "api-versions",
    "cluster-info",
    "events",
    "wait",
];

/// `kubectl <object> <sub>` inspection forms; mutating object commands
/// (use-context, rollout undo/restart, apply, create, delete, …) stay out.
pub(crate) const KUBECTL_READ_OBJECTS: &[(&str, &[&str])] = &[
    (
        "config",
        &["view", "get-contexts", "current-context", "get-clusters"],
    ),
    ("auth", &["can-i", "whoami"]),
    ("rollout", &["status", "history"]),
];

/// Flags whose following token is a value (namespace, output, filter, …).
/// Used only to find the real subcommand; unknown flags make the lookup
/// conservative (the next token is treated as the subcommand → not read-only).
pub(crate) const COMMAND_VALUE_FLAGS: &[&str] = &[
    "-n", "--namespace", "-c", "--container", "-o", "--output", "-l", "--selector", "-f",
    "--filter", "--field-selector", "--sort-by", "--tail", "--since", "--until", "--name",
    "--format", "--context", "-H", "--host", "--config", "--log-level", "--kubeconfig",
    "--cluster", "--user", "--server", "--token", "--as", "--as-group", "--request-timeout",
    "--chunk-size",
];

/// git inspection subcommands; `branch`/`tag`/`remote`/`reflog` are further
/// narrowed to their listing shapes by [`git_segment_risk`].
pub(crate) const GIT_READ_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "blame",
    "describe",
    "rev-parse",
    "remote",
    "tag",
    "reflog",
];

/// Second-level `ip` object commands that mutate network state even though
/// the object itself (`route`, `link`, `addr`, …) inspects by default.
pub(crate) const MUTATING_OBJECT_COMMANDS: &[&str] = &[
    "add", "del", "delete", "flush", "set", "change", "replace", "append",
];

/// Command prefixes that merely wrap an inner command; unwrapped before the
/// real verb is assessed.
pub(crate) const WRAPPER_VERBS: &[&str] = &[
    "nohup", "timeout", "watch", "time", "nice", "ionice", "stdbuf", "env",
];

/// Wrapper flags that consume the following token as their value
/// (`nice -n 10`, `ionice -c 2`, `watch -n 1`).
pub(crate) const WRAPPER_VALUE_FLAGS: &[&str] = &["-n", "-c", "-p"];

/// Verbs that format or wipe raw disks / power the machine off.
pub(crate) const DESTRUCTIVE_VERBS: &[&str] = &[
    "mkfs",
    "mkfs.ext2",
    "mkfs.ext3",
    "mkfs.ext4",
    "mkfs.xfs",
    "mkfs.btrfs",
    "mkfs.vfat",
    "mkfs.fat",
    "mkswap",
    "fdisk",
    "sfdisk",
    "cfdisk",
    "gdisk",
    "sgdisk",
    "parted",
    "partprobe",
    "wipefs",
    "blkdiscard",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
];

/// SQL interpreters whose arguments may carry a DROP statement.
pub(crate) const SQL_VERBS: &[&str] = &["mysql", "mariadb", "psql", "sqlite3"];

/// System files whose overwrite or deletion is always catastrophic.
pub(crate) const CRITICAL_FILES: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/fstab",
    "/boot/",
];

/// Directory components that hold credentials, key material, or cloud /
/// cluster identity. Any path reaching into them downgrades a whitelisted
/// inspection command to `Unknown`.
pub(crate) const SENSITIVE_COMPONENTS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube"];

/// File basenames that carry credentials or leak them via history.
pub(crate) const SENSITIVE_BASENAMES: &[&str] = &[
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".htpasswd",
    ".pgpass",
    ".my.cnf",
    "my.cnf",
    ".bash_history",
    ".zsh_history",
    ".sh_history",
    ".mysql_history",
    ".psql_history",
];

/// Extensions used by private keys / PKCS containers.
pub(crate) const SENSITIVE_EXTENSIONS: &[&str] = &[".pem", ".key", ".p12", ".pfx"];

/// System credential files (prefix match, so `shadow-` backups match too).
pub(crate) const SENSITIVE_SYSTEM_PREFIXES: &[&str] =
    &["/etc/shadow", "/etc/gshadow", "/etc/sudoers"];

/// sudo flags whose following token is their value (`-u user`, `-g group`).
pub(crate) const SUDO_VALUE_FLAGS: &[&str] = &["-u", "-g", "-p", "-C", "-R", "-T", "-D", "-h"];
