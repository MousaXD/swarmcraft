#!/usr/bin/env python3
"""Verify native Debian Desktop restart and incompatible profile refusal after profile_upgrade.py."""
import argparse, json, os, socket, subprocess, tempfile
from pathlib import Path
import native_desktop_qa as qa
from native_frontend_rollback import isolated_home, require_closed
from profile_upgrade import inventory
# Invoke in a dedicated user/network namespace; do not attach a legacy
# upgraded identity to any live network during this acceptance run.
parser = argparse.ArgumentParser(description=__doc__)
for name in ("artifacts", "candidate", "xvfb", "driver", "native-driver"):
    parser.add_argument("--" + name, type=lambda value: Path(value).resolve(), required=True)
args = parser.parse_args()
qa.require({name for _, name in socket.if_nameindex()} == {"lo"},
           "isolated loopback-only network namespace required")
os.umask(0o077)
lab = args.artifacts
qa.require(lab.stat().st_mode & 0o077 == 0, "private acceptance artifacts required")
evidence = json.loads((lab / "result.json").read_text())
qa.require(all(evidence["checks"].values()) and len(evidence["checks"]) == 10,
           "completed real package upgrade acceptance required")
profile = lab / "upgrade-profile"
binary = args.candidate / "swarmcraft-desktop"
qa.require(qa.sha(args.candidate / "swarmcraft") == evidence["candidate_cli_sha256"]
           and qa.sha(args.candidate / "swarmcraft-import") == evidence["candidate_import_sha256"],
           "candidate must match the package upgrade evidence")
driver = args.driver
native = args.native_driver
qa.WORLD_NAME = "Upgrade acceptance"
result = {"scope":"actual current extracted Debian package, isolated loopback-only network namespace, upgraded real Java save","stages":{}}
try:
    with tempfile.TemporaryDirectory(prefix="swarmcraft-native-52-",dir=lab) as tmp:
        root=Path(tmp)
        display=next(i for i in range(111,160) if not Path(f"/tmp/.X{i}-lock").exists() and not Path(f"/tmp/.X11-unix/X{i}").exists())
        auth=root/"Xauthority"
        auth.touch(mode=0o600)
        qa.checked(["xauth","-f",auth,"add",f":{display}","MIT-MAGIC-COOKIE-1",os.urandom(16).hex()])
        runtime=root/"runtime";runtime.mkdir(mode=0o700)
        os.environ.update(DISPLAY=f":{display}",XAUTHORITY=str(auth),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND="x11",GSETTINGS_BACKEND="memory",WEBKIT_DISABLE_DMABUF_RENDERER="1",WEBKIT_DISABLE_COMPOSITING_MODE="1",LIBGL_ALWAYS_SOFTWARE="1",DBUS_SESSION_BUS_ADDRESS=f"unix:path={root/'no-session-bus'}",GIO_USE_VFS="local",GTK_USE_PORTAL="0",NO_AT_BRIDGE="1")
        os.environ.pop("WAYLAND_DISPLAY",None)
        xvfb=subprocess.Popen([str(args.xvfb),f":{display}","-screen","0","1440x1000x24","-nolisten","tcp","-auth",str(auth)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
        try:
            qa.wait("private Xvfb",lambda:subprocess.run(["xdpyinfo"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0,12)
            with isolated_home(root):
                world=qa.world_from_list(binary,profile)
                qa.require(world is not None,"upgraded world unavailable")
                checkpoint=qa.verified_checkpoint(binary,profile,world)
                qa.require(checkpoint == evidence["original_checkpoint"], "checkpoint differs from upgrade evidence")
                identity=qa.cli(binary,profile,"identity")
                consent=qa.consent(binary,profile,world)
                qa.require(not consent["consented"] and consent["selected_heap_mib"]==0,"not withdrawn")
                data=profile/"data"
                baseline={k:v for k,v in inventory(data).items() if k.startswith("worlds/") and not k.endswith(".lock") or k=="identity/peer.key" or k.endswith("hosting-consent.json") or k=="profile-compatibility.json"}
                for stage in ("boot","restart"):
                    result["active_stage"]=stage
                    with qa.WebDriver(binary,profile,driver,native) as ui:
                        qa.ready(ui,empty=False)
                        qa.select_world(ui,world)
                        qa.require(qa.cli(binary,profile,"identity")==identity and qa.verified_checkpoint(binary,profile,world)==checkpoint,"identity/checkpoint changed")
                        after=qa.consent(binary,profile,world)
                        qa.require(after["consent_revision"]==consent["consent_revision"] and not after["consented"] and after["selected_heap_mib"]==0,"consent changed")
                    require_closed(ui,(binary,))
                    current=inventory(data)
                    qa.require(all(current.get(k)==v for k,v in baseline.items()),"canonical/consent bytes changed")
                    result["stages"][stage]=True
                    print("PASS",stage,flush=True)
                result["active_stage"]="future_profile"
                bad=qa.fresh_profile(root,"future-profile")
                marker=bad/"data/profile-compatibility.json"
                marker.write_text('{"format_schema":1,"minimum_reader_generation":2,"storage_schema_version":1,"protocol_version":1}')
                before=inventory(bad/"data")
                with qa.WebDriver(binary,bad,driver,native) as ui:
                    qa.wait("unsupported profile startup failure",lambda:qa.read_is(ui,qa.FAIL,"Startup failed"),25)
                    banner=ui.read(qa.BANNER)["value"]
                    qa.require("minimum reader generation" in banner.lower() or "incompatible profile" in banner.lower() or "reader generation" in banner.lower(),"no actionable incompatible-profile message")
                    qa.click(ui,qa.RETRY)
                    qa.wait("retry denied",lambda:qa.read_is(ui,qa.FAIL,"Startup failed"),25)
                require_closed(ui,(binary,))
                qa.require(inventory(bad/"data")==before,"unsupported profile mutated")
                result["stages"]["future_profile"]=True
                print("PASS future profile refusal",flush=True)
        finally:
            qa.stop_owned_tree(xvfb)
    result["complete"]=True
    result.pop("active_stage",None)
except Exception as e:
    result["failure"]={"category":type(e).__name__,"reason":str(e)[:240]}
    raise
finally:
    qa.write_json(lab/"native-desktop-result.json",result)
