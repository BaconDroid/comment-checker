#!/usr/bin/env python3
'''Flag-rate self-check: arithmetic on three literal payloads.'''

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import replay  # noqa: E402

REPORTS = [
    ('<comments file="a.py">\n\t<comment line-number="1" reason="agent-memo">x</comment>\n</comments>', 2),
    ('<comments file="a.py">\n\t<comment line-number="1" reason="agent-memo">x</comment>\n</comments>', 2),
    ("[check-comments] Success: No problematic comments/docstrings found", 0),
]


def fake_run(binary: str, payloads: list[bytes]) -> dict[str, object]:
    reasons = [replay.parse_reasons(stderr) for stderr, _ in REPORTS]
    return {
        "payloads": len(REPORTS),
        "exit2": sum(code == 2 for _, code in REPORTS),
        "findings": sum(len(items) for items in reasons),
        "per_reason": {"agent-memo": sum(len(items) for items in reasons)},
    }


def main() -> int:
    original = replay.run
    replay.run = fake_run
    try:
        result = replay.run("unused", [b""] * len(REPORTS))
    finally:
        replay.run = original

    rates = replay.flag_rates(result)
    expected = {"agent-memo": round(2 / 3, 4)}
    assert rates == expected, f"rates {rates} != {expected}"
    dropped = replay.flag_rates({**result, "payloads": 99})
    assert dropped["agent-memo"] == round(2 / 99, 4), dropped
    print(f"ok: flag rates {rates} from a 3-payload denominator")
    return 0


if __name__ == "__main__":
    sys.exit(main())
