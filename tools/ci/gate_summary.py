"""Append failed gate steps from Actions' steps context, without a second list of checks."""
import json
import os
from pathlib import Path


def summary(steps: dict) -> str:
    failed = [name for name, result in steps.items() if result["outcome"] == "failure"]
    return "### Failed gate steps\n\n" + ("".join(f"- `{name}`\n" for name in failed) if failed else "None.\n")


if __name__ == "__main__":
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as out:
        out.write(summary(json.loads(os.environ["GATE_STEPS"])))
