import { describe, expect, it } from "vitest";
import {
  CONNECTION_PROVIDER_ID,
  PLUGIN_ID,
  TUNNELS_QUICK_ID,
  TUNNELS_WORKBENCH_ID,
  WORKBENCH_ID,
} from "./pluginIds";

// Locks the constants to the identity/contribution ids in manifest.json.
// Cross-file identity is also covered by scripts/validate_repo.
describe("pluginIds", () => {
  it("matches the manifest identity and contribution ids", () => {
    expect(PLUGIN_ID).toBe("io.dbx.sshx");
    expect(CONNECTION_PROVIDER_ID).toBe("io.dbx.sshx.connection");
    expect(WORKBENCH_ID).toBe("io.dbx.sshx.workbench");
    expect(TUNNELS_WORKBENCH_ID).toBe("io.dbx.sshx.tunnels");
    expect(TUNNELS_QUICK_ID).toBe("io.dbx.sshx.tunnels.quick");
  });
});
