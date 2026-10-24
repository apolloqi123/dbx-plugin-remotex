use super::*;
use crate::features::mcp::safety::{assess_command, CommandRisk};
use serde_json::json;

/// Builds a `NormalizedAlert` directly (bypasses `normalize`).
fn alert(title: &str, message: &str) -> NormalizedAlert {
    NormalizedAlert {
        title: title.to_string(),
        message: message.to_string(),
        ..NormalizedAlert::default()
    }
}

fn classify_text(text: &str) -> Category {
    classify(&alert("", text))
}

// ---------- normalize ----------

#[test]
fn structured_json_fields_are_extracted() {
    let payload = r#"{"alertId":"a-1","title":"CPU High","message":"cpu at 95%",
            "severity":"CRITICAL","source":"Prometheus",
            "data":{"node":"web-1","value":95}}"#;
    let normalized = normalize(payload);
    assert_eq!(normalized.alert_id, "a-1");
    assert_eq!(normalized.title, "CPU High");
    assert_eq!(normalized.message, "cpu at 95%");
    assert_eq!(normalized.severity, "critical"); // lowercased
    assert_eq!(normalized.source, "prometheus");
    // Object data becomes pretty-printed JSON.
    assert!(normalized.data_json.contains("\n"));
    assert!(normalized.data_json.contains("\"node\": \"web-1\""));
}

#[test]
fn snake_case_alert_id_is_accepted() {
    let normalized = normalize(r#"{"alert_id":"sn-7","message":"m"}"#);
    assert_eq!(normalized.alert_id, "sn-7");
}

#[test]
fn non_string_fields_are_missing_and_data_non_object_is_dropped() {
    let normalized = normalize(r#"{"alertId":42,"title":7,"data":"not-an-object"}"#);
    assert_eq!(normalized.alert_id, "");
    assert_eq!(normalized.title, "");
    assert_eq!(normalized.data_json, "");
}

#[test]
fn empty_message_falls_back_to_whole_payload() {
    let payload = r#"{"severity":"critical"}"#;
    let normalized = normalize(payload);
    assert_eq!(normalized.message, payload);
}

#[test]
fn plain_text_payload_becomes_the_message() {
    let normalized = normalize("  Disk almost full on /var  ");
    assert_eq!(normalized.message, "Disk almost full on /var");
    assert_eq!(normalized.title, "");
    assert_eq!(normalized.severity, "unknown");
    assert_eq!(normalized.source, "");
    assert_eq!(normalized.data_json, "");
}

#[test]
fn missing_severity_becomes_unknown() {
    assert_eq!(normalize(r#"{"message":"m"}"#).severity, "unknown");
    assert_eq!(
        normalize(r#"{"message":"m","severity":""}"#).severity,
        "unknown"
    );
}

#[test]
fn overlong_fields_are_clamped_by_chars() {
    let payload = json!({
        "title": "t".repeat(3000),
        "message": "m".repeat(3000),
        "data": { "blob": "x".repeat(20000) },
    })
    .to_string();
    let normalized = normalize(&payload);
    assert_eq!(normalized.title.chars().count(), MAX_FIELD_CHARS);
    assert_eq!(normalized.message.chars().count(), MAX_FIELD_CHARS);
    assert_eq!(normalized.data_json.chars().count(), MAX_DATA_JSON_CHARS);
    // Multibyte content truncates on char boundaries.
    let wide = normalize(&json!({ "message": "内".repeat(3000) }).to_string());
    assert_eq!(wide.message.chars().count(), MAX_FIELD_CHARS);
}

// ---------- classify ----------

#[test]
fn cpu_alerts_are_detected() {
    assert_eq!(classify_text("CPU 使用率过高, 负载 12.0"), Category::Cpu);
    assert_ne!(classify_text("free memory is low"), Category::Cpu);
}

#[test]
fn memory_alerts_are_detected() {
    assert_eq!(classify_text("内存使用 95%, swap 已满"), Category::Memory);
    assert_ne!(classify_text("cpu idle 100%"), Category::Memory);
}

#[test]
fn disk_alerts_are_detected() {
    assert_eq!(
        classify_text("磁盘告警: no space left on device"),
        Category::Disk
    );
    assert_ne!(classify_text("memory usage 90%"), Category::Disk);
}

#[test]
fn inode_alerts_are_detected() {
    assert_eq!(classify_text("inode usage 99% on /var"), Category::Inode);
    // Plain disk keywords must not bleed into Inode.
    assert_ne!(classify_text("disk io slow"), Category::Inode);
}

#[test]
fn network_alerts_are_detected() {
    assert_eq!(
        classify_text("网卡 eth0 packet loss 10%, 丢包"),
        Category::Network
    );
    assert_ne!(classify_text("cpu load high"), Category::Network);
}

#[test]
fn oom_alerts_are_detected() {
    assert_eq!(
        classify_text("Out of memory: OOM killer terminated java"),
        Category::Oom
    );
    assert_ne!(classify_text("disk almost full"), Category::Oom);
}

/// Pins the tie-break decision: OOM keywords that also score a Memory
/// keyword ("out of memory" → "memory", "内存耗尽" → "内存") must resolve
/// to Oom, i.e. Oom precedes Memory in the tie-break order.
#[test]
fn oom_outranks_memory_on_tied_scores() {
    assert_eq!(classify_text("内存耗尽"), Category::Oom);
    assert_eq!(classify_text("out of memory"), Category::Oom);
}

#[test]
fn service_alerts_are_detected() {
    assert_eq!(
        classify_text("systemd: unit nginx.service restart failed"),
        Category::Service
    );
    assert_ne!(classify_text("cpu high"), Category::Service);
}

#[test]
fn zero_scores_fall_back_to_generic() {
    assert_eq!(
        classify_text("backup finished successfully at 03:00"),
        Category::Generic
    );
    assert_eq!(classify(&NormalizedAlert::default()), Category::Generic);
}

// ---------- playbook ----------

/// D6 core acceptance: every emitted suggestion — for every category,
/// including the service group with a substituted unit name and the
/// service→generic fallback — must be provably read-only.
#[test]
fn every_suggestion_passes_the_read_only_whitelist() {
    let service_alert = alert("服务异常", "systemd unit nginx.service restart failed");
    let cases = [
        Category::Cpu,
        Category::Memory,
        Category::Disk,
        Category::Inode,
        Category::Network,
        Category::Oom,
        Category::Service,
        Category::Generic,
    ];
    for category in cases {
        let suggestions = playbook(category, &service_alert);
        assert!(
            !suggestions.is_empty(),
            "{category:?} produced no suggestions"
        );
        for item in suggestions {
            assert_eq!(
                assess_command(&item.command),
                CommandRisk::ReadOnly,
                "command not read-only: {}",
                item.command
            );
            assert!(!item.command.contains('>'));
            assert!(!item.command.contains("sudo"));
            assert!(!item.command.contains("$("));
            assert!(!item.command.contains('`'));
        }
    }
    // Service with an unextractable unit also falls back to generic.
    let anonymous = alert("", "service crashed, no unit name");
    for item in playbook(Category::Service, &anonymous) {
        assert_eq!(assess_command(&item.command), CommandRisk::ReadOnly);
    }
}

#[test]
fn purpose_keys_are_stable() {
    let empty = NormalizedAlert::default();
    // The Service row needs a sample unit name; an empty alert would
    // (correctly) fall back to the generic command set.
    let named = alert("服务异常", "systemd unit cron.service restart failed");
    let keys = |category| -> Vec<&'static str> {
        let sample = if category == Category::Service {
            &named
        } else {
            &empty
        };
        playbook(category, sample)
            .into_iter()
            .map(|item| item.purpose_key)
            .collect()
    };
    assert_eq!(
        keys(Category::Cpu),
        ["loadSnapshot", "cpuTopProcesses", "cpuVmstat"]
    );
    assert_eq!(keys(Category::Memory), ["memFree", "memTopProcesses"]);
    assert_eq!(keys(Category::Disk), ["diskUsage", "diskDu"]);
    assert_eq!(keys(Category::Inode), ["diskInode"]);
    assert_eq!(keys(Category::Network), ["netSummary", "netLinks"]);
    assert_eq!(keys(Category::Oom), ["oomDmesg", "oomJournal"]);
    assert_eq!(keys(Category::Service), ["serviceStatus", "serviceJournal"]);
    assert_eq!(
        keys(Category::Generic),
        ["loadSnapshot", "diskUsage", "memFree", "cpuTopProcesses"]
    );
}

// ---------- extract_service ----------

#[test]
fn extracts_name_dot_service_token() {
    let found = alert("", "Failed to start nginx.service: timeout");
    assert_eq!(extract_service(&found).as_deref(), Some("nginx.service"));
}

#[test]
fn extracts_unit_key_form() {
    let found = alert("", "unit=redis failed to start");
    assert_eq!(extract_service(&found).as_deref(), Some("redis"));
}

#[test]
fn extracts_service_key_form() {
    let found = alert("", "service=mysql stopped unexpectedly");
    assert_eq!(extract_service(&found).as_deref(), Some("mysql"));
}

#[test]
fn missing_service_returns_none() {
    assert_eq!(extract_service(&alert("", "unknown failure")), None);
    assert_eq!(extract_service(&alert("", "unit= failed")), None);
}

/// D6 defense: a sensitive-path candidate must not reach
/// `systemctl status <svc>` (it would leave the read-only class).
#[test]
fn sensitive_candidates_are_rejected() {
    let key_like = alert("", "unit=id_rsa.service failed");
    assert_eq!(extract_service(&key_like), None);
    let hidden = alert("", "check unit=.ssh for details");
    assert_eq!(extract_service(&hidden), None);
}

// ---------- triage + triage_view ----------

#[test]
fn category_names_are_stable() {
    assert_eq!(Category::Cpu.name(), "cpu");
    assert_eq!(Category::Memory.name(), "memory");
    assert_eq!(Category::Disk.name(), "disk");
    assert_eq!(Category::Inode.name(), "inode");
    assert_eq!(Category::Network.name(), "network");
    assert_eq!(Category::Oom.name(), "oom");
    assert_eq!(Category::Service.name(), "service");
    assert_eq!(Category::Generic.name(), "generic");
}

#[test]
fn triage_view_uses_camel_case_response_shape() {
    let payload = r#"{"alertId":"a-9","title":"CPU 高","severity":"critical","data":{"v":1}}"#;
    let result = triage(payload);
    let view = triage_view(&result);

    // Top-level keys exactly as contracted.
    let top_keys: Vec<&str> = view
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(top_keys, ["category", "normalized", "suggestions"]);

    let normalized = &view["normalized"];
    assert_eq!(normalized["alertId"], "a-9");
    assert_eq!(normalized["title"], "CPU 高");
    // Empty parsed message degrades to the whole payload.
    assert_eq!(normalized["message"], payload);
    assert_eq!(normalized["severity"], "critical");
    assert_eq!(normalized["source"], "");
    assert!(normalized["dataJson"]
        .as_str()
        .unwrap()
        .contains("\"v\": 1"));

    assert_eq!(view["category"], "cpu");
    assert_eq!(
        view["suggestions"][0],
        json!({ "command": "uptime", "purposeKey": "loadSnapshot" })
    );
    assert_eq!(view["suggestions"][2]["purposeKey"], "cpuVmstat");
}

#[test]
fn triage_end_to_end_uses_the_substituted_service_name() {
    let payload = r#"{"title":"服务异常","message":"systemd unit cron.service failed"}"#;
    let result = triage(payload);
    assert_eq!(result.category, Category::Service);
    let commands: Vec<&str> = result
        .suggestions
        .iter()
        .map(|item| item.command.as_str())
        .collect();
    assert_eq!(
        commands,
        [
            "systemctl status cron.service",
            "journalctl -u cron.service -n 100 --no-pager",
        ]
    );
}

/// 扩展意图分类矩阵（原 mcp 工具面用例迁移；#203 工具瘦身把
/// `ssh_alert_triage` 移出 MCP 目录，分类逻辑本身保留给
/// `ssh/alert/triage` sidecar 方法）。
#[test]
fn classifies_extended_intents_with_read_only_suggestions() {
    let cases: [(&str, &str); 9] = [
        // (payload, 期望 category)
        (
            r#"{"alertId":"n1","message":"network eth0 packet loss 5%, bandwidth saturated"}"#,
            "network",
        ),
        // 中英混合：丢包 + bandwidth 双命中仍归 network。
        (
            r#"{"message":"网络抖动, 丢包严重, bandwidth 不足"}"#,
            "network",
        ),
        // OOM 优先于 memory（out of memory 同时命中 memory 关键词）。
        (
            r#"{"message":"Out of memory: oom-kill killed process 4321 (java)"}"#,
            "oom",
        ),
        (
            r#"{"message":"systemd unit cron.service restart failed"}"#,
            "service",
        ),
        // 零命中回落 generic。
        (
            r#"{"message":"backup job completed without errors at 03:00"}"#,
            "generic",
        ),
        // 中英混合 CPU。
        (r#"{"message":"CPU loadavg 飙升, 处理器过热"}"#, "cpu"),
        (r#"{"message":"内存 swap 已满, mem usage 99%"}"#, "memory"),
        (r#"{"message":"磁盘 no space left on device"}"#, "disk"),
        (r#"{"message":"inode usage 98% on /var"}"#, "inode"),
    ];
    for (payload, expected) in cases {
        let view = triage_view(&triage(payload));
        assert_eq!(view["category"], expected, "payload: {payload}");
        let suggestions = view["suggestions"].as_array().unwrap();
        assert!(!suggestions.is_empty(), "payload: {payload}");
        for item in suggestions {
            let command = item["command"].as_str().unwrap();
            assert_eq!(
                assess_command(command),
                CommandRisk::ReadOnly,
                "suggestion not read-only: {command}"
            );
            assert!(!command.contains('>'));
            assert!(!command.contains("sudo"));
            assert!(!command.contains("$("));
        }
    }
}
