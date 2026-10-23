#!/usr/bin/env python3
"""Small helpers shared by the live/offline smoke scripts.

Every smoke script bootstraps ``scripts/smoke/lib`` onto ``sys.path`` (the
same place ``sidecar_client`` lives), so this module is imported as a plain
top-level module::

    from smoke_common import auto_accept_challenge, fail, missing_method, step

Only genuinely identical copies live here. Scripts whose reporting or
connection semantics differ keep their local helper (e.g. extra state
bookkeeping or a different ``fail`` signature).
"""

from __future__ import annotations

import re
import sys
from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover - typing only, avoids a runtime import
    from sidecar_client import SidecarClient


def step(name: str) -> None:
    """Print one smoke section header."""
    print(f"\n==> {name}")


def fail(message: str, client: "SidecarClient | None" = None) -> None:
    """Close the client (when given), print a FAIL line and exit non-zero."""
    if client:
        client.close()
    print(f"\nFAIL: {message}", file=sys.stderr)
    sys.exit(1)


def auto_accept_challenge(event: dict) -> dict | None:
    """Auto-accept host-key challenges while a request is in flight."""
    return _host_key_resolution(event, verbose=True)


def auto_accept_challenge_quiet(event: dict) -> dict | None:
    """Same as :func:`auto_accept_challenge` without the per-challenge log."""
    return _host_key_resolution(event, verbose=False)


def _host_key_resolution(event: dict, *, verbose: bool) -> dict | None:
    if event.get("method") != "connection/challenge":
        return None
    params = event.get("params", {}).get("params") or event.get("params", {})
    # Intentional divergence from the three former local copies folded into
    # this helper: they answered every connection/challenge, sending
    # challengeId=None for a malformed event. Ignoring it here is deliberate —
    # a null id only yields a backend rejection and an obscure error in the
    # requesting call, while leaving the challenge pending surfaces as the
    # request's own timeout, which is the honest signal that the event shape
    # is wrong.
    if "challengeId" not in params:
        return None
    if verbose:
        print(f"    host-key challenge: {params.get('keyType')} "
              f"{str(params.get('fingerprint'))[:32]}...")
    return {
        "method": "ssh/host-key/resolve",
        "params": {
            "challengeId": params["challengeId"],
            "operationId": params.get("operationId"),
            "accept": True,
            "remember": True,
        },
    }


def missing_method(error: Exception) -> str | None:
    """Return the unregistered method name if the error means 'Method not found'."""
    text = str(error)
    if "Method not found" not in text and "-32601" not in text:
        return None
    match = re.search(r"Method not found:\s*([\w./-]+)", text)
    return match.group(1) if match else ""
