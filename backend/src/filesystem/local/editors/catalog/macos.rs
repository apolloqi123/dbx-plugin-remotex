use super::*;

pub(super) fn extend(push: PushEntry<'_>, env: &EnvPaths, probe: &dyn Fn(&Path) -> bool) {
    let app = |dir: &str| probe_app(env, probe, dir);
    push(
        "vscode",
        "Visual Studio Code",
        open_app("Visual Studio Code"),
        app("Visual Studio Code"),
        text_extensions(),
    );
    push(
        "sublime",
        "Sublime Text",
        open_app("Sublime Text"),
        app("Sublime Text"),
        text_extensions(),
    );
    push(
        "textedit",
        "TextEdit",
        open_app("TextEdit"),
        app("TextEdit") || probe(&PathBuf::from("/System/Applications/TextEdit.app")),
        text_extensions(),
    );
    push(
        "wps",
        "WPS Office",
        open_app("wpsoffice"),
        app("wpsoffice"),
        &[".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"],
    );
    push(
        "msword",
        "Microsoft Word",
        open_app("Microsoft Word"),
        app("Microsoft Word"),
        &[".doc", ".docx", ".rtf"],
    );
    push(
        "msexcel",
        "Microsoft Excel",
        open_app("Microsoft Excel"),
        app("Microsoft Excel"),
        &[".xls", ".xlsx", ".csv"],
    );
    push(
        "msppt",
        "Microsoft PowerPoint",
        open_app("Microsoft PowerPoint"),
        app("Microsoft PowerPoint"),
        &[".ppt", ".pptx"],
    );
    push(
        "libreoffice",
        "LibreOffice",
        open_app("LibreOffice"),
        app("LibreOffice"),
        OFFICE_EXTS,
    );
}
