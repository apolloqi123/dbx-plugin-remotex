use super::*;

pub(super) fn extend(
    push: PushEntry<'_>,
    env: &EnvPaths,
    probe: &dyn Fn(&Path) -> bool,
    lister: &dyn Fn(&Path) -> Vec<PathBuf>,
) {
    // CreateProcess cannot exec `code.cmd`, so only real .exe install
    // locations are probed (the two standard per-user/machine paths).
    let vscode = first_existing(
        &[
            env.local_appdata
                .as_ref()
                .map(|d| d.join(r"Programs\Microsoft VS Code\Code.exe")),
            env.program_files
                .as_ref()
                .map(|d| d.join(r"Microsoft VS Code\Code.exe")),
        ],
        probe,
    );
    push(
        "vscode",
        "Visual Studio Code",
        exe_spec(vscode.as_deref()),
        vscode.is_some(),
        text_extensions(),
    );
    let notepad = env
        .windows_dir
        .as_ref()
        .map(|d| d.join(r"System32\notepad.exe"))
        .filter(|path| probe(path));
    push(
        "notepad",
        "Notepad",
        exe_spec(notepad.as_deref()),
        notepad.is_some(),
        text_extensions(),
    );
    let notepadpp = first_existing(
        &[
            env.program_files
                .as_ref()
                .map(|d| d.join(r"Notepad++\notepad++.exe")),
            env.local_appdata
                .as_ref()
                .map(|d| d.join(r"Programs\Notepad++\notepad++.exe")),
        ],
        probe,
    );
    push(
        "notepadpp",
        "Notepad++",
        exe_spec(notepadpp.as_deref()),
        notepadpp.is_some(),
        text_extensions(),
    );
    let sublime = first_existing(
        &[
            env.program_files
                .as_ref()
                .map(|d| d.join(r"Sublime Text\sublime_text.exe")),
            env.local_appdata
                .as_ref()
                .map(|d| d.join(r"Programs\Sublime Text\sublime_text.exe")),
        ],
        probe,
    );
    push(
        "sublime",
        "Sublime Text",
        exe_spec(sublime.as_deref()),
        sublime.is_some(),
        text_extensions(),
    );
    // WPS Windows installs into a versioned directory:
    // <root>\Kingsoft\WPS Office\<version>\office6\{wps,et,wpp}.exe
    let wps_candidates = [
        env.local_appdata
            .as_ref()
            .map(|d| d.join(r"Kingsoft\WPS Office")),
        env.program_files_x86
            .as_ref()
            .map(|d| d.join(r"Kingsoft\WPS Office")),
    ];
    let wps_roots: Vec<&PathBuf> = wps_candidates.iter().flatten().collect();
    let wps = |binary: &'static str| -> Option<PathBuf> {
        wps_roots
            .iter()
            .find_map(|root| probe_versioned(root, &["office6", binary], probe, lister))
    };
    let writer = wps("wps.exe");
    push(
        "wps-writer",
        "WPS Writer",
        exe_spec(writer.as_deref()),
        writer.is_some(),
        &[".doc", ".docx", ".rtf"],
    );
    let sheet = wps("et.exe");
    push(
        "wps-spreadsheet",
        "WPS Spreadsheets",
        exe_spec(sheet.as_deref()),
        sheet.is_some(),
        &[".xls", ".xlsx", ".csv", ".et"],
    );
    let slides = wps("wpp.exe");
    push(
        "wps-presentation",
        "WPS Presentation",
        exe_spec(slides.as_deref()),
        slides.is_some(),
        &[".ppt", ".pptx", ".dps"],
    );
    let office = |binary: &'static str| -> Option<PathBuf> {
        first_existing(
            &[
                env.program_files
                    .as_ref()
                    .map(|d| d.join(format!(r"Microsoft Office\root\Office16\{binary}"))),
                env.program_files_x86
                    .as_ref()
                    .map(|d| d.join(format!(r"Microsoft Office\root\Office16\{binary}"))),
            ],
            probe,
        )
    };
    let word = office("WINWORD.EXE");
    push(
        "msword",
        "Microsoft Word",
        exe_spec(word.as_deref()),
        word.is_some(),
        &[".doc", ".docx", ".rtf"],
    );
    let excel = office("EXCEL.EXE");
    push(
        "msexcel",
        "Microsoft Excel",
        exe_spec(excel.as_deref()),
        excel.is_some(),
        &[".xls", ".xlsx", ".csv"],
    );
    let ppt = office("POWERPNT.EXE");
    push(
        "msppt",
        "Microsoft PowerPoint",
        exe_spec(ppt.as_deref()),
        ppt.is_some(),
        &[".ppt", ".pptx"],
    );
    let libre = first_existing(
        &[env
            .program_files
            .as_ref()
            .map(|d| d.join(r"LibreOffice\program\soffice.exe"))],
        probe,
    );
    push(
        "libreoffice",
        "LibreOffice",
        exe_spec(libre.as_deref()),
        libre.is_some(),
        OFFICE_EXTS,
    );
}
