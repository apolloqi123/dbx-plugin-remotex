use super::*;
use CommandRisk::{Destructive, ReadOnly, Unknown};

fn read_only(command: &str) {
    assert_eq!(
        assess_command(command),
        ReadOnly,
        "expected ReadOnly: {command}"
    );
}

fn unknown(command: &str) {
    assert_eq!(
        assess_command(command),
        Unknown,
        "expected Unknown: {command}"
    );
}

fn destructive(command: &str) {
    assert!(
        matches!(assess_command(command), Destructive(_)),
        "expected Destructive: {command}"
    );
}

#[test]
fn inspection_commands_are_read_only() {
    read_only("df -h");
    read_only("ls -la /var/log");
    read_only("cat /etc/hostname");
    read_only("systemctl status nginx");
    read_only("systemctl is-active sshd");
    read_only("journalctl -u sshd -n 50 --no-pager");
    read_only("docker ps -a");
    read_only("docker logs --tail 100 c1");
    read_only("git log --oneline -5");
    read_only("ps aux | grep mysqld | grep -v grep");
    read_only("ps aux 2>&1 | grep sshd");
    read_only("du -xh --max-depth=1 /www | sort -rh | head -10");
    read_only("echo hello");
    read_only("free -m && uptime");
    read_only("ip a");
    read_only("ip addr show eth0");
    read_only("crontab -l");
    read_only("service nginx status");
}

#[test]
fn docker_and_kubectl_inspection_forms_are_read_only() {
    // Nested object commands used by the AI's server audits.
    read_only("docker node ls");
    read_only("docker node ps node-1");
    read_only("docker node inspect node-1");
    read_only("docker service ls");
    read_only("docker service ps api");
    read_only("docker service logs --tail 100 api");
    read_only("docker network ls");
    read_only("docker network inspect bridge");
    read_only("docker volume ls");
    read_only("docker volume inspect data");
    read_only("docker stack services app");
    read_only("docker stack config -c stack.yml");
    read_only("docker system df -v");
    read_only("docker compose ps");
    read_only("docker compose logs --tail 50");
    read_only("docker container ls");
    read_only("docker image ls");
    read_only("docker --context prod ps");
    // Mutating object verbs stay Unknown (graded mode prompts).
    unknown("docker service update --image nginx:1.27 api");
    unknown("docker node update --availability drain node-1");
    unknown("docker network create app-net");
    unknown("docker volume rm data");
    unknown("docker stack deploy -c stack.yml app");
    unknown("docker compose up -d");
    unknown("docker restart api");
    // kubectl reads, including flags before the verb.
    read_only("kubectl -n prod get pods");
    read_only("kubectl --context prod get nodes");
    read_only("kubectl config view --minify");
    read_only("kubectl config get-contexts");
    read_only("kubectl auth can-i get pods");
    read_only("kubectl rollout status deployment/api");
    read_only("kubectl cluster-info");
    unknown("kubectl config use-context prod");
    unknown("kubectl rollout restart deployment/api");
    unknown("kubectl apply -f app.yaml");
    unknown("kubectl delete pod api-0");
    unknown("kubectl -n prod exec -it api-0 -- sh");
}

#[test]
fn runs_under_sudo_detects_inline_privilege_escalation() {
    assert!(super::runs_under_sudo("sudo whoami"));
    assert!(super::runs_under_sudo("sudo -u postgres psql -l"));
    assert!(super::runs_under_sudo("echo hi && sudo reboot"));
    assert!(super::runs_under_sudo("FOO=1 sudo id"));
    assert!(!super::runs_under_sudo("echo sudo is a word here"));
    assert!(!super::runs_under_sudo("whoami"));
    assert!(!super::runs_under_sudo("ls && cat /etc/hostname"));
}

#[test]
fn whitelisted_output_flags_are_unknown() {
    // `sort -o` / `find -fprint…` write files while keeping read-only verbs.
    unknown("sort -o /etc/cron.d/x /tmp/in");
    unknown("sort --output=/etc/cron.d/x /tmp/in");
    unknown("sort -o/etc/cron.d/x /tmp/in");
    read_only("sort /tmp/in | head -5");
    unknown("find / -fprint /tmp/keys");
    unknown("find / -fprintf /tmp/x '%p'");
    unknown("find . -fls /tmp/out");
    read_only("find /var/log -name '*.log'");
    read_only("du -sh /var | sort -rh | head");
}

#[test]
fn deep_subcommand_mutations_are_unknown() {
    unknown("ip link set dev eth0 down");
    unknown("ip route flush all");
    unknown("ip addr add 10.0.0.1/24 dev eth0");
    unknown("ip neigh flush all");
    unknown("ip rule add from 0.0.0.0 table 200");
    read_only("ip link show eth0");
    read_only("ip route get 8.8.8.8");
    read_only("ip addr show dev eth0");

    unknown("git branch -D main");
    unknown("git branch feature");
    unknown("git tag -d v1");
    unknown("git tag v1.0");
    unknown("git remote add evil https://example.test/x.git");
    unknown("git remote set-url origin https://example.test/x.git");
    unknown("git reflog delete --all");
    read_only("git branch -a");
    read_only("git branch --show-current");
    read_only("git tag -l 'v*'");
    read_only("git remote -v");
    read_only("git remote show origin");
    read_only("git reflog");
    read_only("git reflog show");
    read_only("git log --oneline -5");
    read_only("git show HEAD~1");

    unknown("dmesg -C");
    unknown("dmesg -c");
    unknown("dmesg -n 1");
    unknown("dmesg --console-level emerg");
    read_only("dmesg | tail -20");
    read_only("dmesg -T");

    unknown("history -c");
    unknown("history -w");
    unknown("history -d 5");
    read_only("history");
    read_only("history 10");
}

#[test]
fn sensitive_paths_downgrade_read_only() {
    unknown("cat ~/.ssh/id_rsa");
    unknown("cat /root/.ssh/id_ed25519");
    unknown("ls -la /root/.ssh");
    unknown("grep -r key /home/u/.ssh");
    unknown("cat /etc/shadow");
    unknown("grep root /etc/shadow-");
    unknown("stat /etc/sudoers");
    unknown("cat /srv/app/.env.production");
    unknown("cat /root/.aws/credentials");
    unknown("find / -name id_rsa*");
    unknown("cat /etc/ssl/private/server.pem");
    unknown("tail -5 /root/.bash_history");
    unknown("du /home/u/.gnupg");
    read_only("cat ~/.ssh/id_rsa.pub");
    read_only("cat /etc/hostname");
    read_only("cat /etc/ssh/sshd_config");
    read_only("ls /etc");
    read_only("grep error /var/log/app.log");
    read_only("df -h");
}

#[test]
fn mutating_or_unclear_commands_are_unknown() {
    unknown("rm -rf /home/vagrant/acct-main");
    unknown("systemctl restart nginx");
    unknown("systemctl stop mysqld");
    unknown("echo x > /tmp/out.txt");
    unknown("cat /etc/passwd >> /tmp/copy");
    unknown("sed -i s/a/b/ file");
    unknown("awk '{print $1}' file");
    unknown("mysql -e 'UPDATE t SET x=1'");
    unknown("sudo cat /var/log/auth.log");
    unknown("env FOO=1 rm -rf /home/vagrant/junk");
    unknown("echo $(whoami)");
    unknown("crontab -e");
    unknown("crontab -r");
    unknown("journalctl --vacuum-size=100M");
    unknown("find /var -name '*.log' -exec rm {} +");
}

#[test]
fn catastrophic_patterns_are_destructive() {
    destructive("rm -rf /");
    destructive("rm -rf /*");
    destructive("rm -fr /etc");
    destructive("rm -rf /etc/nginx");
    destructive("rm --recursive /usr/local");
    destructive("rm -rf ~");
    destructive("rm -rf $HOME");
    destructive("rm -rf .");
    destructive("rm -rf *");
    destructive("rm -f /etc/passwd");
    destructive("mkfs.ext4 /dev/sda1");
    destructive("wipefs -a /dev/sdb");
    destructive("dd if=/dev/zero of=/dev/sda");
    destructive("dd if=img.raw of=/dev/nvme0n1 bs=4M");
    destructive("echo x > /dev/sda");
    destructive("shutdown -h now");
    destructive("reboot");
    destructive("init 0");
    destructive("telinit 6");
    destructive(":(){ :|:& };:");
    destructive("chmod -R 777 /");
    destructive("chmod -R 755 /etc");
    destructive("chown -R nobody /usr");
    destructive("echo hack >> /etc/sudoers");
    destructive("truncate -s 0 /etc/shadow");
    destructive("mysql -e 'DROP DATABASE prod'");
    destructive("psql -c 'drop table users'");
    destructive("docker system prune -af");
    destructive("find / -delete");
    destructive("kill -9 -1");
    destructive("rm -rf / ; echo done");
    destructive("uptime && mkfs.xfs /dev/vdb");
}

#[test]
fn routine_cleanup_stays_out_of_the_destructive_class() {
    // Real-world cleanup commands must not demand confirmation.
    unknown("rm -rf /root/.cache/*");
    unknown("rm -rf /lib/modules/5.4.0-117-generic");
    unknown("truncate -s 0 /www/server/data/vagrant.err");
    unknown("apt-get clean");
    unknown("journalctl --rotate && sleep 3");
    unknown("rm -rf /tmp/dbx-mcp-smoke-dir");
    unknown("dd if=/dev/zero of=/dev/null bs=1M count=10");
}

#[test]
fn wrappers_and_assignments_are_unwrapped_conservatively() {
    read_only("nohup df -h");
    read_only("timeout 30 du -sh /var");
    read_only("nice -n 10 ls /");
    read_only("watch -n 1 uptime");
    read_only("FOO=bar BAR=baz df -h");
    destructive("timeout 10 rm -rf /data");
    destructive("FOO=bar rm -rf /data");
    unknown("timeout 10 systemctl restart nginx");
}

// —— 第五轮对抗面：灾难门的绕过尝试 ————————————————

/// Reliability round 5: the catastrophic gate must hold across the
/// classic evasion shapes — quoting, leading whitespace, combined
/// commands, sudo prefixes, `sh -c` scripts, and `$()`/backtick
/// substitution payloads (`echo $(rm -rf /)` used to classify as a
/// plain Unknown and run unconfirmed on writable connections).
#[test]
fn destructive_gate_survives_evasion_variants() {
    // Quoting and whitespace.
    destructive("\"rm\" -rf /");
    destructive("'rm' -fr /etc");
    destructive("rm -rf \"/\"");
    destructive("  \t rm  -rf  /");
    // Combined commands / chains.
    destructive("cd /tmp && rm -rf /");
    destructive("cd /tmp; echo hi; rm -rf /");
    destructive("rm -rf / || echo done");
    destructive("echo start && mkfs.ext4 /dev/vdb");
    // sudo prefix variants (inner command must be unwrapped).
    destructive("sudo rm -rf /");
    destructive("sudo rm -rf /etc/nginx");
    destructive("sudo -u root rm -rf /etc");
    destructive("sudo sudo rm -rf /");
    destructive("sudo -- mkfs.ext4 /dev/sda");
    destructive("env sudo rm -rf /");
    destructive("FOO=1 sudo rm -rf /");
    destructive("nohup sudo shutdown -h now");
    destructive("timeout 5 sudo dd if=/dev/zero of=/dev/sda");
    // Shell -c scripts.
    destructive("sh -c 'rm -rf /'");
    destructive("bash -c \"mkfs.ext4 /dev/sda1\"");
    destructive("bash -lc 'reboot'");
    destructive("sudo sh -c 'rm -rf /'");
    destructive("sh -c 'echo start; rm -rf /'");
    // Substitution payloads.
    destructive("echo $(rm -rf /)");
    destructive("echo `rm -rf /`");
    destructive("echo $(date; rm -rf /)");
    destructive("echo $(echo $(rm -rf /))");
    destructive("echo \"$(shutdown -h now)\"");
    destructive("run=$(reboot)");
    // Variable-expansion targets stay covered (existing shapes, plus
    // quoted variants the trim step normalizes).
    destructive("rm -rf \"$HOME\"");
    destructive("rm -rf '${HOME}'");
}

/// The flip side of the evasion sweep: substitution, wrappers and shell
/// -c invocations must never UPGRADE anything to ReadOnly — the
/// whitelist is only reachable by a directly-recognized inspection
/// command. Anything not destructive inside them stays Unknown.
#[test]
fn substitution_and_wrappers_never_upgrade_to_read_only() {
    unknown("$(df -h)");
    unknown("echo $(df -h)");
    unknown("`df -h`");
    unknown("echo `date`");
    unknown("echo $(whoami)");
    unknown("grep \"$(date)\" /var/log/app.log");
    unknown("sudo systemctl restart nginx");
    unknown("sudo cat /var/log/auth.log");
    unknown("sudo -u postgres psql -l");
    unknown("sh -c 'df -h'");
    unknown("bash -c 'echo hi'");
    unknown("sh");
    unknown("bash --norc");
    // The read-only whitelist itself is untouched by the sweep.
    read_only("df -h");
    read_only("ls -la /var/log");
    read_only("ps aux | grep mysqld");
}

/// Reliability round 5: the sensitive-path denylist must hold across
/// path-shape aliasing — `~` expansions, `./` segments, double slashes,
/// and bare system basenames from a relative cwd. URL-encoded tokens
/// are intentionally NOT decoded: neither the shell nor SFTP decode
/// percent escapes, so `%2e%2e/x` is a literal filename, not a
/// traversal — while `%2e%2e/.ssh` is flagged only because it still
/// contains a literal `.ssh` component. Pinned so a future decoder
/// cannot silently change this behavior.
#[test]
fn sensitive_path_detection_survives_path_shape_evasion() {
    for token in [
        "~/.ssh/id_rsa",
        "./.ssh/id_rsa",
        ".//.ssh/id_ed25519",
        "//root//.ssh//id_rsa",
        "~root/.ssh/id_rsa",
        "~/../root/.ssh/id_rsa",
        "/root/./.aws/credentials",
        "//etc//shadow",
        "/etc/./shadow",
        "/etc//gshadow",
        "./etc/shadow", // relative from a system cwd
        "shadow",       // bare basename (`cat shadow` in /etc)
        "/etc/sudoers.d/override",
        "/etc/./sudoers",
        "./id_rsa",
        "sub/../.netrc",
        "/srv/app/./.env.production",
        "/etc/ssl/private//server.pem",
        "%2e%2e/.ssh", // literal `.ssh` component, not percent decoding
    ] {
        assert!(
            is_sensitive_path(token),
            "expected '{token}' to be flagged sensitive"
        );
    }
    for token in [
        "/etc/hostname",
        "/etc/ssh/sshd_config", // config, not a host key
        "/var/log/app.log",
        "/home/u/project/shadowing.md", // basename must match exactly
        "%2e%2e%2fnotes.txt",           // percent-encoded literals are NOT decoded…
        "%2e%2e/x",                     // …and carry no literal sensitive component
        "id_rsa.pub",                   // public half
    ] {
        assert!(
            !is_sensitive_path(token),
            "expected '{token}' NOT to be flagged sensitive"
        );
    }
}

#[test]
fn sensitive_path_detection_handles_windows_separators() {
    for token in [
        r"C:\Users\运维\.ssh\id_ed25519",
        r"C:\Users\运维\.aws\credentials",
        r"\\server\share\etc\shadow",
    ] {
        assert!(
            is_sensitive_path(token),
            "expected Windows path '{token}' to be flagged sensitive"
        );
    }
    assert!(!is_sensitive_path(r"C:\work\project\shadowing.md"));
}

#[test]
fn quoting_is_not_parsed_and_stays_conservative() {
    // A `;` inside quotes splits segments; both halves must be read-only
    // for the whole command to pass. Here the second half (`b' x ...`)
    // is unrecognized, so the fd-dup neutralization cannot save it —
    // exactly the fail-closed behavior we want.
    unknown("grep -F 'a;b' x 2>&1 || true");
    read_only("grep -F a.b x 2>&1 || true");
    unknown("grep -F 'a;b' /etc/hosts"); // second half: `b' /etc/hosts`
    read_only("echo \"line1\"; df -h");
    unknown("echo \"x\"; systemctl restart nginx");
}

#[test]
fn destructive_classification_names_the_pattern() {
    assert!(matches!(assess_command("reboot"), Destructive(_)));
    assert_eq!(assess_command("df -h"), ReadOnly);
    assert_eq!(
        assess_command("rm -rf /etc"),
        Destructive("recursive rm targets a system root")
    );
}

// —— A2: terminal input assessment ————————————————

#[test]
fn control_only_detection_matches_normalized_content() {
    // Pure control sequences (Enter, Ctrl+C). Per the §1.2 contract the
    // check is strictly per-character `is_control`, so CSI arrow runs
    // ("\u{1b}[A") carry printable bytes and are refused (fail closed) —
    // an operator can still send them through a writable connection.
    assert!(is_control_only_input("\r"));
    assert!(is_control_only_input("\u{3}"));
    assert!(is_control_only_input(""));
    assert!(!is_control_only_input("\u{1b}[A"));
    // Any visible text is not control-only.
    assert!(!is_control_only_input("y"));
    assert!(!is_control_only_input("echo hi\r"));
    assert!(!is_control_only_input("exit\n"));
}

#[test]
fn terminal_input_risk_takes_the_max_across_enter_lines() {
    // Plain inspection text stays Low (read-only-ish: allowed on normal
    // connections, still refused as visible text on read-only ones via
    // the control-only rule).
    assert_eq!(assess_terminal_input("echo hi\r"), CommandRisk::ReadOnly);
    assert_eq!(
        assess_terminal_input("df -h\recho done\r"),
        CommandRisk::ReadOnly
    );
    // A mutating line is Unknown...
    assert_eq!(
        assess_terminal_input("systemctl restart nginx\r"),
        CommandRisk::Unknown
    );
    // ...and a catastrophic line is Destructive regardless of position.
    assert!(matches!(
        assess_terminal_input("echo start\rrm -rf /data\r"),
        CommandRisk::Destructive(_)
    ));
    // An empty input has no risk.
    assert_eq!(assess_terminal_input(""), CommandRisk::ReadOnly);
}

#[test]
fn terminal_input_normalization_caps_at_8kib_and_strips_nul() {
    // CRLF and LF both fold to CR; NUL bytes are stripped.
    assert_eq!(normalize_terminal_input("a\r\nb\nc\u{0}d"), "a\rb\rcd");
    // Oversized input truncates at 8 KiB on a char boundary.
    let oversized = "x".repeat(9 * 1024);
    let normalized = normalize_terminal_input(&oversized);
    assert_eq!(normalized.len(), 8 * 1024);
    // Multi-byte chars are never split mid-codepoint.
    let multibyte = "é".repeat(5000); // 2 bytes each
    let normalized = normalize_terminal_input(&multibyte);
    assert!(normalized.len() <= 8 * 1024);
    assert_eq!(normalized.chars().count(), 4096);
}
