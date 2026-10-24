//! Native file/folder pickers backed by the OS dialogs.

use super::*;

/// 系统对话框 single-flight：SDK worker 池 ≥2 线程时，第二次「导入」会并发
/// 再弹一个叠着的原生对话框（宿主 timeout_ms 只管放弃等待，管不住已弹出的
/// 窗口）。RAII 释放覆盖取消/报错/panic 全部路径。
static DIALOG_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub(super) struct DialogFlightGuard;

impl DialogFlightGuard {
    pub(super) fn acquire() -> Result<Self, String> {
        DIALOG_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| DialogFlightGuard)
            .map_err(|_| {
                // 守卫命中时对话框多半还开着（Windows 上可能被主窗挡住）：
                // 文案必须给出出路，而不是一条死路的报错（#160）。
                "A native file dialog is already open — finish or cancel it first (it may be behind this window), then retry".to_string()
            })
    }
}

impl Drop for DialogFlightGuard {
    fn drop(&mut self) {
        DIALOG_IN_FLIGHT.store(false, Ordering::Release);
    }
}

/// AppleScript 双引号字符串字面量转义：`"` 与 `\` 在文件名中均合法，不转义
/// 会让 osascript 编译失败乃至注入 AppleScript 语句（osascript 是代码执行
/// 原语）。仅被平台对话框引用，其余平台死代码——与 FilePickFilter 同一
/// 处理：结构性 suppress，不加 cfg。
#[allow(dead_code)]
pub(super) fn applescript_quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        if ch == '"' || ch == '\\' {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted
}

/// PowerShell 单引号字符串字面量（含成对引号）：内部单引号双写 `''`。
#[allow(dead_code)]
pub(super) fn powershell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Opens the OS-native file picker (defaulting to `~/.ssh` where the platform
/// allows it) and returns the chosen absolute path; `None` when the user
/// cancels. Used by the connection form's "import private key" action — the
/// host-rendered form cannot pick files itself.
pub fn pick_file() -> Result<Option<String>, String> {
    let _flight = DialogFlightGuard::acquire()?;
    let ssh_dir = home_subdir(".ssh");
    trim_picked(pick_file_dialog(ssh_dir.as_deref(), None))
}

/// File-picker filter for one import source. Each platform honors the field it
/// can: Windows/Linux filter on `extensions`, macOS on `macos_types` (custom
/// extensions have no UTI and stay unfiltered there, relying on the explicit
/// type step ahead of the dialog). Each field is only read behind its
/// platform cfg, so unused-on-this-platform is structural — suppressed here,
/// not per field.
#[allow(dead_code)]
pub struct FilePickFilter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub macos_types: &'static [&'static str],
}

impl FilePickFilter {
    pub(super) fn from_source(source: &ImportSourceSpec) -> Self {
        Self {
            name: source.label,
            extensions: source.extensions,
            macos_types: source.macos_types,
        }
    }

    #[allow(dead_code)]
    fn has_extensions(&self) -> bool {
        !self.extensions.is_empty()
    }

    // 唯一调用点在 macOS 门控的 pick_file_dialog 里；其余平台为本方法死代码
    // （与 has_extensions 同一处理）。
    #[allow(dead_code)]
    fn has_macos_types(&self) -> bool {
        !self.macos_types.is_empty()
    }
}

/// Native "pick one from a list" dialog; returns the picked label, `None` on
/// cancel. Used by the import source selection before the file dialog.
#[cfg(windows)]
pub(super) fn pick_source_dialog(labels: &[&str]) -> Result<Option<String>, String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    // WinForms 单选列表（复用 OpenFileDialog 同款 STA + EncodedCommand 管线）：
    // 第一项默认选中，OK 返回选中项文本，关闭窗口静默取消。
    let mut script = String::from(concat!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;",
        "Add-Type -AssemblyName System.Windows.Forms;",
        "$f=New-Object System.Windows.Forms.Form;",
        "$f.Text='Import connections';$f.Width=380;$f.Height=400;",
        "$f.FormBorderStyle='FixedDialog';$f.MaximizeBox=$false;$f.MinimizeBox=$false;$f.TopMost=$true;$f.StartPosition='CenterScreen';",
        "$l=New-Object System.Windows.Forms.Label;",
        "$l.Text='Choose the source of the sessions to import:';$l.SetBounds(12,10,340,20);$f.Controls.Add($l);",
        "$radios=@();",
    ));
    for (index, label) in labels.iter().enumerate() {
        script.push_str(&format!(
            "$r{index}=New-Object System.Windows.Forms.RadioButton;$r{index}.Text={text};$r{index}.SetBounds(16,{y},340,24);$f.Controls.Add($r{index});$radios+=$r{index};",
            index = index,
            y = 40 + index * 30,
            text = powershell_quote(label),
        ));
    }
    script.push_str("$radios[0].Checked=$true;");
    script.push_str(concat!(
        "$ok=New-Object System.Windows.Forms.Button;$ok.Text='OK';$ok.DialogResult='OK';$ok.SetBounds(270,360,80,26);$f.Controls.Add($ok);$f.AcceptButton=$ok;",
        "if($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK){$sel=$radios | Where-Object {$_.Checked} | Select-Object -First 1;[Console]::Out.Write($sel.Text)}",
    ));
    let mut utf16le = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-STA",
            "-EncodedCommand",
            &BASE64_STANDARD.encode(utf16le),
        ])
        .output()
        .map_err(|error| format!("Failed to launch the source picker: {error}"))?;
    let label = String::from_utf8_lossy(&output.stdout)
        .trim_matches(['\r', '\n'])
        .to_string();
    Ok(if label.is_empty() { None } else { Some(label) })
}

#[cfg(not(windows))]
pub(super) fn pick_source_dialog(labels: &[&str]) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        // AppleScript choose from list：取消输出空串/`false`，选中输出该项文本。
        let list = labels
            .iter()
            .map(|label| format!("\"{}\"", applescript_quote(label)))
            .collect::<Vec<_>>()
            .join(",");
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "choose from list {{{list}}} with title \"Import connections\" with prompt \"Choose the source of the sessions to import:\""
            ))
            .output()
            .map_err(|error| format!("Failed to launch the source picker: {error}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let label = String::from_utf8_lossy(&output.stdout)
            .trim_matches(['\r', '\n'])
            .to_string();
        Ok((!label.is_empty() && label != "false").then_some(label))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // zenity 列表显示 label；kdialog radiolist 用 kind 作 tag、label 作标题，
        // 两者输出都映射回 label 交给统一解析。
        for program in ["zenity", "kdialog"] {
            let mut command = std::process::Command::new(program);
            if program == "zenity" {
                command
                    .arg("--list")
                    .arg("--title=Import connections")
                    .arg("--text=Choose the source of the sessions to import:")
                    .arg("--column=Source");
                for label in labels {
                    command.arg(label);
                }
            } else {
                command
                    .arg("--radiolist")
                    .arg("Choose the source of the sessions to import:")
                    .arg("--title=Import connections");
                for (index, label) in labels.iter().enumerate() {
                    command
                        .arg(if index == 0 {
                            label.to_string()
                        } else {
                            format!("{index}")
                        })
                        .arg(label)
                        .arg(if index == 0 { "on" } else { "off" });
                }
            }
            match command.output() {
                Ok(output) => {
                    if !output.status.success() {
                        return Ok(None);
                    }
                    let text = String::from_utf8_lossy(&output.stdout)
                        .trim_matches(['\r', '\n'])
                        .to_string();
                    if text.is_empty() {
                        return Ok(None);
                    }
                    // zenity 原样返回 label；kdialog 返回 tag（label 或序号）。
                    if labels.contains(&text.as_str()) {
                        return Ok(Some(text));
                    }
                    if let Ok(index) = text.parse::<usize>() {
                        if let Some(label) = labels.get(index) {
                            return Ok(Some(label.to_string()));
                        }
                    }
                    return Ok(None);
                }
                Err(_) => continue,
            }
        }
        Err("No source picker available (install zenity or kdialog)".to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = labels;
        Err("Source picker is not available on this platform".to_string())
    }
}

/// Parameterized native file picker shared by the private-key import (no
/// filter, `~/.ssh` default) and the per-source import connections picker.
#[cfg(windows)]
pub(super) fn pick_file_dialog(
    default_dir: Option<&Path>,
    filter: Option<&FilePickFilter>,
) -> Result<Option<String>, String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    // OpenFileDialog 需要 STA；默认目录不存在时退回用户目录；
    // -EncodedCommand 传 base64(UTF-16LE) 规避引号转义，输出 UTF-8。
    let mut script = String::from(concat!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;",
        "Add-Type -AssemblyName System.Windows.Forms;",
        "$d=New-Object System.Windows.Forms.OpenFileDialog;",
    ));
    match default_dir {
        Some(dir) => script.push_str(&format!(
            "$d.InitialDirectory={};",
            powershell_quote(&dir.to_string_lossy())
        )),
        None => script.push_str("$d.InitialDirectory=$env:USERPROFILE;"),
    }
    match filter {
        Some(filter) if filter.has_extensions() => {
            let (name, extensions) = (filter.name, filter.extensions);
            let patterns = extensions
                .iter()
                .map(|extension| format!("*.{extension}"))
                .collect::<Vec<_>>()
                .join(";");
            // Filter 整体是 PowerShell 单引号字符串：name/patterns 里的
            // `'` 需双写转义，否则脚本编译失败乃至注入。
            script.push_str(&format!(
                "$d.Filter={};",
                powershell_quote(&format!(
                    "{name} ({patterns})|{patterns}|All files (*.*)|*.*"
                )),
            ));
        }
        _ => script.push_str("$d.Filter='All files (*.*)|*.*';"),
    }
    // OpenFileDialog 无 TopMost 属性：以一个不可见置顶 owner 表单挂靠
    // （ShowDialog(owner) 继承 owner 的 Z 序），否则对话框常弹在 DBX 主窗
    // 后面——用户以为导入没执行而重试，撞上 single-flight 守卫报
    // "already open"（#160 的完整复现链）。
    script.push_str(concat!(
        "Add-Type -AssemblyName System.Drawing;",
        "$o=New-Object System.Windows.Forms.Form;",
        "$o.TopMost=$true;$o.ShowInTaskbar=$false;$o.FormBorderStyle='None';$o.Size=New-Object System.Drawing.Size(1,1);$o.StartPosition='Manual';$o.Location=New-Object System.Drawing.Point(-2000,-2000);",
        "$o.Show();",
        "if($d.ShowDialog($o) -eq [System.Windows.Forms.DialogResult]::OK){[Console]::Out.Write($d.FileName)};",
        "$o.Close();",
    ));
    let mut utf16le = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-STA",
            "-EncodedCommand",
            &BASE64_STANDARD.encode(utf16le),
        ])
        .output()
        .map_err(|error| format!("Failed to launch the file picker: {error}"))?;
    let path = String::from_utf8_lossy(&output.stdout)
        .trim_matches(['\r', '\n'])
        .to_string();
    Ok(if path.is_empty() { None } else { Some(path) })
}

#[cfg(target_os = "macos")]
pub(super) fn pick_file_dialog(
    default_dir: Option<&Path>,
    filter: Option<&FilePickFilter>,
) -> Result<Option<String>, String> {
    // 默认定位 `default_dir`（缺省用户主目录）；用户取消时非零退出。
    // `of type` 只接受 UTI：自定义扩展名（.mxtsessions 等）没有 UTI，
    // 对应来源不过滤，靠前置的类型选择步骤引导。
    let mut command = std::process::Command::new("osascript");
    command.arg("-e").arg(format!(
        "set defaultDir to {}",
        match default_dir {
            // `"` 与 `\` 在路径中合法：不转义会让 osascript 编译失败乃至
            // 注入 AppleScript 语句（osascript 是代码执行原语）。
            Some(dir) => format!(
                "POSIX file \"{}\"",
                applescript_quote(&dir.to_string_lossy())
            ),
            None => "path to home folder".to_string(),
        }
    ));
    command.arg("-e").arg(format!(
        "POSIX path of (choose file default location defaultDir{})",
        match filter {
            Some(filter) if filter.has_macos_types() => {
                let macos_types = filter.macos_types;
                format!(
                    " of type {{{}}}",
                    macos_types
                        .iter()
                        .map(|uti| format!("\"{uti}\""))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            _ => String::new(),
        }
    ));
    let output = command
        .output()
        .map_err(|error| format!("Failed to launch the file picker: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .trim_matches(['\r', '\n'])
        .to_string();
    Ok(if path.is_empty() { None } else { Some(path) })
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn pick_file_dialog(
    default_dir: Option<&Path>,
    filter: Option<&FilePickFilter>,
) -> Result<Option<String>, String> {
    let fallback = std::env::var_os("HOME").map(PathBuf::from);
    let default = default_dir.or(fallback.as_deref());
    let zenity_default = default
        .map(|dir| format!("{}/", dir.to_string_lossy()))
        .unwrap_or_default();
    let mut command = std::process::Command::new("zenity");
    command.arg("--file-selection");
    if !zenity_default.is_empty() {
        command.arg(format!("--filename={zenity_default}"));
    }
    if let Some(filter) = filter {
        if filter.has_extensions() {
            let (name, extensions) = (filter.name, filter.extensions);
            let patterns = extensions
                .iter()
                .map(|extension| format!("*.{extension}"))
                .collect::<Vec<_>>()
                .join(" ");
            command.arg(format!("--file-filter={name} | {patterns}"));
        }
    }
    if let Ok(output) = command.output() {
        if !output.status.success() {
            return Ok(None);
        }
        let path = String::from_utf8_lossy(&output.stdout)
            .trim_matches(['\r', '\n'])
            .to_string();
        return Ok(if path.is_empty() { None } else { Some(path) });
    }
    // zenity 不在时回落 kdialog（Qt 过滤器语法：pattern 空格分隔、`|` 后标题）。
    let kdialog_dir = default
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let mut command = std::process::Command::new("kdialog");
    command.arg("--getopenfilename").arg(&kdialog_dir);
    if let Some(filter) = filter {
        // Qt 过滤器语法：单条目内多 pattern 空格分隔——与 zenity 同一扩展名
        // 集合，不再只取第一个（同功能跨平台过滤范围一致）。
        if !filter.extensions.is_empty() {
            let patterns = filter
                .extensions
                .iter()
                .map(|extension| format!("*.{extension}"))
                .collect::<Vec<_>>()
                .join(" ");
            command.arg(format!("{patterns}|{}", filter.name));
        }
    }
    match command.output() {
        Ok(output) => {
            if !output.status.success() {
                return Ok(None);
            }
            let path = String::from_utf8_lossy(&output.stdout)
                .trim_matches(['\r', '\n'])
                .to_string();
            Ok(if path.is_empty() { None } else { Some(path) })
        }
        Err(_) => Err("No file picker available (install zenity or kdialog)".to_string()),
    }
}
