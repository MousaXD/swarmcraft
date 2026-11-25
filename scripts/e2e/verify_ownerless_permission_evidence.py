#!/usr/bin/env python3
"""Replay retained #65 observations and verify the public genesis signatures."""
import argparse
import json
from pathlib import Path
import tempfile

from ownerless_permission_smoke import parts, verify_observations, verify_profile
from region_runtime import require


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt",type=Path)
    parser.add_argument("--cli",type=Path,required=True)
    parser.add_argument("--expected-world",required=True)
    args=parser.parse_args()
    data=json.loads(args.receipt.read_text())
    stats=verify_observations(data)
    require(data["world_id"]==args.expected_world,"unexpected evidence world")
    with tempfile.TemporaryDirectory(prefix="sc65-verify-") as temp:
        root=Path(temp)
        cert=root/"certificate.json"
        cert.write_text(json.dumps(data["creation_certificate"]))
        binding=verify_profile(args.cli,cert,args.expected_world,parts(data["binding"])[2],root/"unused-profile")
        require(binding==data["binding"],"signature pin differs")
    print(json.dumps(stats,indent=2))


if __name__=="__main__":
    main()
