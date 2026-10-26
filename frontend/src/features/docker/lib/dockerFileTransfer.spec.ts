import { describe, expect, it } from "vitest";
import {
  bytesToBase64,
  containerTargetParams,
  isFileTooLarge,
  parseExportResponse,
  stagingDirectory,
  stagingFileName,
} from "./dockerFileTransfer";

describe("container transfer helpers", () => {
  it("splits a staging path into directory and file name", () => {
    expect(stagingDirectory("/tmp/dbx-sshx.abc/report.log")).toBe("/tmp/dbx-sshx.abc");
    expect(stagingFileName("/tmp/dbx-sshx.abc/report.log")).toBe("report.log");
    expect(stagingDirectory("/tmp/x")).toBe("/tmp");
    expect(stagingFileName("/tmp/x")).toBe("x");
  });

  it("encodes chunk bytes as standard base64", () => {
    expect(bytesToBase64(new Uint8Array([]))).toBe("");
    expect(bytesToBase64(new TextEncoder().encode("abc"))).toBe("YWJj");
    expect(bytesToBase64(new Uint8Array([0, 255, 16]))).toBe("AP8Q");
  });

  it("enforces the 512 MiB single-file limit", () => {
    const limit = 512 * 1024 * 1024;
    expect(isFileTooLarge(limit)).toBe(false);
    expect(isFileTooLarge(limit + 1)).toBe(true);
    expect(isFileTooLarge(0)).toBe(false);
    expect(isFileTooLarge(undefined)).toBe(false);
  });

  it("normalizes the export response defensively", () => {
    expect(
      parseExportResponse({
        stagingPath: "/tmp/dbx-sshx.1/report.log",
        size: 3,
        name: "report.log",
        location: "local",
      }),
    ).toEqual({ stagingPath: "/tmp/dbx-sshx.1/report.log", size: 3, name: "report.log", location: "local" });
    // Unknown location falls back to the SSH staging lane.
    expect(parseExportResponse({ stagingPath: "/tmp/dbx-sshx.1/a" })).toMatchObject({
      location: "remote",
      name: "a",
      size: 0,
    });
    expect(parseExportResponse({ size: 3 })).toBeUndefined();
    expect(parseExportResponse(null)).toBeUndefined();
  });

  it("maps the session to an SSH target and absence to the local daemon", () => {
    expect(containerTargetParams("s1")).toEqual({ sessionId: "s1" });
    expect(containerTargetParams(undefined)).toEqual({ target: "local" });
    expect(containerTargetParams("")).toEqual({ target: "local" });
  });
});
