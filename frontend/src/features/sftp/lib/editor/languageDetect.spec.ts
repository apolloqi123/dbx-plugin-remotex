import { describe, expect, it } from "vitest";
import { languages } from "@codemirror/language-data";
import {
  detectEditorLanguage,
  extensionOf,
  isLogFileName,
  shebangLanguage,
  sniffContent,
} from "./languageDetect";

function nameOf(fileName: string, text = ""): string | undefined {
  return detectEditorLanguage(fileName, text, languages).language?.name;
}

describe("extensionOf", () => {
  it("ignores dotfiles and returns lowercase suffixes", () => {
    expect(extensionOf("app.TS")).toBe("ts");
    expect(extensionOf(".bashrc")).toBe("");
    expect(extensionOf("Makefile")).toBe("");
    expect(extensionOf("/srv/a/b.conf")).toBe("conf");
  });
});

describe("isLogFileName", () => {
  it("recognizes log file conventions", () => {
    expect(isLogFileName("app.log")).toBe(true);
    expect(isLogFileName("syslog.1")).toBe(false);
    expect(isLogFileName("syslog")).toBe(true);
    expect(isLogFileName("messages")).toBe(true);
    expect(isLogFileName("access_log")).toBe(true);
    expect(isLogFileName("nohup.out")).toBe(true);
    expect(isLogFileName("notes.txt")).toBe(false);
  });
});

describe("shebangLanguage", () => {
  it("maps interpreters, including env and version suffixes", () => {
    expect(shebangLanguage("#!/bin/bash\necho hi")).toBe("Shell");
    expect(shebangLanguage("#!/usr/bin/env python3.11\nprint(1)")).toBe("Python");
    expect(shebangLanguage("#!/usr/bin/node\nconsole.log(1)")).toBe("JavaScript");
    expect(shebangLanguage("#! /usr/bin/env node")).toBe("JavaScript");
    expect(shebangLanguage("echo hi")).toBeUndefined();
  });
});

describe("detectEditorLanguage", () => {
  it("keeps language-data extensions and filename patterns", () => {
    expect(nameOf("main.go")).toBe("Go");
    expect(nameOf("script.ts")).toBe("TypeScript");
    expect(nameOf("README.md")).toBe("Markdown");
    expect(nameOf("Dockerfile")).toBe("Dockerfile");
    expect(nameOf("nginx.conf")).toBe("Nginx");
    expect(nameOf("Jenkinsfile")).toBe("Groovy");
    expect(nameOf("Gemfile")).toBe("Ruby");
  });

  it("maps extension-less convention names", () => {
    expect(nameOf("Dockerfile.prod")).toBe("Dockerfile");
    expect(nameOf("Containerfile")).toBe("Dockerfile");
    expect(nameOf(".bashrc")).toBe("Shell");
    expect(nameOf(".zshrc")).toBe("Shell");
    expect(nameOf(".env")).toBe("Shell");
    expect(nameOf("prod.env")).toBe("Shell");
    expect(nameOf("app.service")).toBe("Properties files");
    expect(nameOf("backup.timer")).toBe("Properties files");
    expect(nameOf(".gitconfig")).toBe("Properties files");
    expect(nameOf("hosts")).toBe("Properties files");
  });

  it("maps extra extensions and sniffs generic config", () => {
    expect(nameOf("data.jsonc")).toBe("JSON");
    expect(nameOf("icon.plist")).toBe("XML");
    expect(nameOf("window.xaml")).toBe("XML");
    expect(nameOf("run.ps1")).toBe("PowerShell");
    expect(nameOf("server.conf", "server {\n  listen 80;\n}\n")).toBe("Nginx");
    expect(nameOf("app.cfg", "a=1\nb=2\n")).toBe("Properties files");
  });

  it("falls back to shebang for extension-less scripts", () => {
    expect(nameOf("deploy", "#!/usr/bin/env bash\nset -e")).toBe("Shell");
    expect(nameOf("tool", "#!/usr/bin/python3\nprint(1)")).toBe("Python");
  });

  it("sniffs structured content without a usable extension", () => {
    expect(nameOf("data", '{"a":1,"b":[2,3]}')).toBe("JSON");
    expect(nameOf("bundle", "x: 1\ny: 2\nz: 3\n")).toBe("YAML");
    expect(nameOf("app.ini", "[main]\nkey=value\n")).toBe("Properties files");
    expect(nameOf("feed", '<?xml version="1.0"?><rss/>')).toBe("XML");
    expect(nameOf("page", "<!DOCTYPE html><html></html>")).toBe("HTML");
    expect(nameOf("change", "diff --git a/x b/x\n--- a/x\n+++ b/x\n")).toBe("diff");
    expect(nameOf("query", "SELECT id FROM users WHERE id = 1;")).toBe("SQL");
  });

  it("marks log files and log-shaped unknown content", () => {
    expect(detectEditorLanguage("app.log", "", languages)).toEqual({ log: true });
    expect(detectEditorLanguage("debug", "INFO boot\nWARN slow\nERROR boom", languages)).toEqual({
      log: true,
    });
    expect(
      detectEditorLanguage(
        "records",
        "2026-10-29 12:00:00 start\n2026-10-29 12:00:01 ready\n2026-10-29 12:00:02 done",
        languages,
      ).log,
    ).toBe(true);
  });

  it("prefers a real language over log sniffing", () => {
    const detected = detectEditorLanguage("data.json", "INFO fake\nERROR fake", languages);
    expect(detected.log).toBe(false);
    expect(detected.language?.name).toBe("JSON");
  });

  it("leaves prose and source snippets plain", () => {
    expect(detectEditorLanguage("notes.txt", "hello world\nsecond line", languages)).toEqual({ log: false });
    expect(detectEditorLanguage("README", "This is a note about errors and warnings.", languages)).toEqual({
      log: false,
    });
    expect(detectEditorLanguage("", "", languages)).toEqual({ log: false });
  });
});

describe("sniffContent", () => {
  it("does not treat two random key lines as YAML", () => {
    expect(sniffContent("Note: hello\nAnother: thing\nplain body text")).toEqual({});
  });

  it("stops at the first structural match", () => {
    expect(sniffContent('{"a":1}')).toEqual({ language: "JSON" });
  });
});
