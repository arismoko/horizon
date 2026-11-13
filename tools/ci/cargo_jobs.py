"""Print how many crates a Dyna runner builds at once: its CPU quota, bounded by its memory.

The runner's cgroup (v2) holds its CPU quota and memory limit. Each concurrent rustc may need
`--job-mib` of memory. Four stays the floor: the fixed cap these runners used before.
"""
import argparse
import os
import sys
from pathlib import Path

FLOOR = 4


def limits(cgroup: Path) -> tuple[int, int]:
    """The runner's CPUs and memory bytes; an unlimited value falls back to the machine's."""
    quota, period = (cgroup / "cpu.max").read_text().split()
    cpus = len(os.sched_getaffinity(0)) if quota == "max" else max(1, int(quota) // int(period))
    memory = (cgroup / "memory.max").read_text().strip()
    total = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE") if memory == "max" else int(memory)
    return cpus, total


def jobs(cpus: int, memory: int, job_mib: int) -> int:
    return max(FLOOR, min(cpus, memory // (job_mib << 20)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job-mib", type=int, required=True, help="memory one concurrent rustc may need")
    parser.add_argument("--cgroup", type=Path, default=Path("/sys/fs/cgroup"))
    args = parser.parse_args()
    cpus, memory = limits(args.cgroup)
    chosen = jobs(cpus, memory, args.job_mib)
    print(f"runner cpus={cpus} memory_mib={memory >> 20} job_mib={args.job_mib} jobs={chosen}", file=sys.stderr)
    print(chosen)
