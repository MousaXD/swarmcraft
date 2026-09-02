from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:120]!r}")
    p.write_text(text.replace(old, new, count))


test = "crates/swarm-cli/tests/live_join_replication.rs"
replace(test, "        mode: EpochMode::Solo,\n", "        mode: EpochMode::Quorum,\n")
replace(
    test,
    "        reason: \"live join acceptance seed\".into(),\n",
    "        reason: \"live join quorum-of-one seed\".into(),\n",
)
