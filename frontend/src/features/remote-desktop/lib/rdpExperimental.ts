/** RDP gate: the persisted preference must be exactly `true` to enable it. */
export function rdpExperimentalEnabled(value: unknown): boolean {
  return value === true;
}
