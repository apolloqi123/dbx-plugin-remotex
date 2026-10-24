use super::*;

pub(super) fn extend(push: PushEntry<'_>, env: &EnvPaths, probe: &dyn Fn(&Path) -> bool) {
    // Linux/other: PATH lookup, direct argv launch (no shell).
    let mut cli = |id: &'static str,
                   name: &'static str,
                   binary: &'static str,
                   exts: &'static [&'static str]| {
        let found = probe_on_path(binary, env, probe);
        // 非 Windows 平台的 exe 展示路径统一 POSIX 分隔符：Path::join
        // 的分隔符随构建宿主变化（Windows 上 join 会往 POSIX 风格目录
        // 里插 '\'）。探测用原始 join 结果，落进 payload 前归一化。
        let normalized = found.map(|path| PathBuf::from(path.to_string_lossy().replace('\\', "/")));
        push(
            id,
            name,
            exe_spec(normalized.as_deref()),
            normalized.is_some(),
            exts,
        );
    };
    cli("vscode", "Visual Studio Code", "code", text_extensions());
    cli("sublime", "Sublime Text", "subl", text_extensions());
    cli("gedit", "gedit", "gedit", text_extensions());
    cli("kate", "Kate", "kate", text_extensions());
    cli("libreoffice", "LibreOffice", "soffice", OFFICE_EXTS);
    cli("wps-writer", "WPS Writer", "wps", &[".doc", ".docx"]);
    cli(
        "wps-spreadsheet",
        "WPS Spreadsheets",
        "et",
        &[".xls", ".xlsx", ".csv", ".et"],
    );
    cli(
        "wps-presentation",
        "WPS Presentation",
        "wpp",
        &[".ppt", ".pptx"],
    );
}
