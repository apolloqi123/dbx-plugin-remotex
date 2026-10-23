//! MCP execution helpers.

use super::*;

impl McpState {
    /// `ssh_multi_exec` (IMPL_PLAN_NETCATTY A2-T4): aggregate execution
    /// across several saved connections on the hidden channel (never routes
    /// through the visible terminal or agentTerminalMode). Targets are
    /// resolved and scope-gated up front (any out-of-scope target refuses
    /// the whole call); per-target gates share the call's
    /// `confirmDestructive`. Sequential mode honors `stopOnError`.
    pub(crate) async fn ssh_multi_exec_tool(&self, arguments: &Value) -> Result<Value, String> {
        // Enumerate every missing required parameter in one error (round 6):
        // an absent targets/command pair is reported together so the caller
        // does not burn one round per gap. Wrong-typed values fall through to
        // the precise per-parameter errors below.
        missing_required(arguments, &["targets", "command"])?;
        let command = required_str(arguments, "command")?;
        let targets = arguments
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                "targets must be an array of 1-10 connection references \
                 (connectionId or connectionName strings)"
                    .to_string()
            })?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| "targets must be non-empty strings".to_string())
            })
            .collect::<Result<Vec<String>, String>>()?;
        let parallel = multi_exec::normalize_targets(&targets)?;
        let mode = arguments
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("parallel");
        let sequential = match mode {
            "parallel" => false,
            "sequential" => true,
            other => {
                return Err(format!(
                    "mode must be \"parallel\" or \"sequential\"; got '{other}'"
                ))
            }
        };
        let stop_on_error = arg_bool(arguments, "stopOnError")?.unwrap_or(false);
        let timeout_secs =
            arg_u64(arguments, "timeoutSecs")?.map(|secs| Duration::from_secs(secs.clamp(5, 300)));
        let confirm_destructive = arg_bool(arguments, "confirmDestructive")?.unwrap_or(false);
        let read_only = self.connection_is_read_only(arguments).await;
        // Per-target gates are arguments-only (command text), so they run
        // once here and apply to every dial; the per-connection read-only
        // flag is enforced per dial in `multi_exec_one`. No sudo allowlist
        // lookup: `command_gate` refuses `sudo …` outright (escalation goes
        // through single-target `ssh_exec_sudo`).
        multi_exec::command_gate(command, read_only, confirm_destructive)?;

        // Resolve every raw selector into its registry identity up front so
        // an unknown/ambiguous reference refuses the whole call before any
        // dial (IMPL_PLAN §1.1: 全量归一化 + 作用域门在执行前)。§1.3 作用域：
        // 任一目标越界整体拒绝。
        let scope = self.effective_scope();
        let mut targets_resolved: Vec<multi_exec::TargetRef> = Vec::with_capacity(parallel.len());
        for raw in &parallel {
            let mut selector = json!({ "connectionId": raw });
            let resolved = match self.registered_connection_by_ref(&selector).await? {
                Some(stored) => stored,
                None => {
                    selector = json!({ "connectionName": raw });
                    self.registered_connection_by_ref(&selector)
                        .await?
                        .ok_or_else(|| format!("No saved connection matched '{raw}'"))?
                }
            };
            if scope.is_active()
                && !scope.allows(&resolved.id, resolved.name.as_deref(), &resolved.host)
            {
                return Err(format!(
                    "Connection '{}' ({} / {}) is outside this MCP server's connectionScope; \
                     remove it from targets or ask the operator to widen the scope",
                    resolved.id, resolved.host, resolved.username
                ));
            }
            let payload = json!({
                "id": resolved.id,
                "host": resolved.host,
                "port": resolved.port,
                "username": resolved.username,
            });
            match multi_exec::target_from_connection(raw, &payload) {
                Some(target) => targets_resolved.push(target),
                None => return Err(format!("Connection '{raw}' registry entry is malformed")),
            }
        }

        let mut results: Vec<Value> = Vec::with_capacity(targets_resolved.len());
        if sequential {
            for target in &targets_resolved {
                let row = self.multi_exec_one(target, command, timeout_secs).await;
                let failed = row.get("ok").and_then(Value::as_bool) != Some(true);
                results.push(row);
                if failed && stop_on_error {
                    break;
                }
            }
        } else {
            let mut runs = Vec::with_capacity(targets_resolved.len());
            for target in &targets_resolved {
                runs.push(self.multi_exec_one(target, command, timeout_secs));
            }
            // join! 的参数个数是静态的，动态 1–10 路扇出用递归 helper。
            let rows = join_all(runs).await;
            for row in rows {
                results.push(row);
            }
        }
        let sent = results
            .iter()
            .filter(|row| row.get("ok").and_then(Value::as_bool) == Some(true))
            .count();
        let failed = results.len() - sent;
        Ok(json!({
            "ok": failed == 0,
            "sent": sent,
            "failed": failed,
            "results": results,
        }))
    }

    /// One target row of `ssh_multi_exec`: resolves the saved connection
    /// (already normalized by `normalize_targets`), dials through the shared
    /// pool, and runs the command on the hidden channel. Failures become an
    /// `ok: false` row instead of failing the aggregate call.
    pub(crate) async fn multi_exec_one(
        &self,
        target: &multi_exec::TargetRef,
        command: &str,
        timeout_secs: Option<Duration>,
    ) -> Value {
        let arguments = json!({ "connectionId": target.connection_id });
        let result: Result<Value, String> = async {
            let stored = self
                .registered_connection_by_ref(&arguments)
                .await?
                .ok_or("Connection is no longer registered")?;
            if stored.read_only {
                return Err("Connection is read-only".to_string());
            }
            let set_env = stored.set_env.clone();
            let connection = self.connection(&arguments).await?;
            let outcome = exec::exec_plain(
                &connection,
                command,
                timeout_secs.unwrap_or(exec::PLAIN_EXEC_TIMEOUT),
                &set_env,
            )
            .await?;
            Ok(json!({ "output": outcome.output, "exitCode": outcome.exit_code }))
        }
        .await;
        match result {
            Ok(outcome) => json!({
                "target": target.raw,
                "connectionId": target.connection_id,
                "host": target.host,
                "port": target.port,
                "username": target.username,
                "ok": true,
                "output": outcome["output"],
                "exitCode": outcome["exitCode"],
            }),
            Err(error) => json!({
                "target": target.raw,
                "connectionId": target.connection_id,
                "host": target.host,
                "port": target.port,
                "username": target.username,
                "ok": false,
                "output": "",
                "exitCode": Value::Null,
                "error": error,
            }),
        }
    }
}
