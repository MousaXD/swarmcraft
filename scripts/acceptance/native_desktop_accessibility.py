#!/usr/bin/env python3
"""Packaged Linux keyboard-only dialog gate; isolated synthetic world, no mock IPC."""
import argparse, json, os, re, subprocess, sys, tempfile
from pathlib import Path
import native_desktop_qa as q
parser = argparse.ArgumentParser(description='Exercise real packaged Linux WebKitGTK keyboard and dialog accessibility on a private Xvfb display.')
parser.add_argument('--packages', type=Path, required=True, help='Directory containing shipping.deb and candidate.deb')
parser.add_argument('--artifacts', type=Path, required=True, help='Private output directory for screenshots and result JSON')
parser.add_argument('--xvfb', type=Path, required=True)
parser.add_argument('--driver', type=Path, required=True)
parser.add_argument('--native-driver', type=Path, required=True)
args = parser.parse_args()
os.umask(0o077)
base = args.packages.resolve()
artifacts = args.artifacts.resolve()
artifacts.mkdir(mode=0o700, parents=True, exist_ok=True)
q.require(artifacts.stat().st_mode & 0o077 == 0, 'Use a private artifact directory')

result = {'scope': 'Linux x86_64 extracted Debian packages; actual WebKitGTK and four bundled Rust sidecars', 'build_base': q.checked(['git','-C',q.REPO,'rev-parse','HEAD']).decode().strip(), 'variants': {}}
def read(ui, script, args=None):
    return ui.call('/execute/sync','POST',{'script':script,'args':args or []})
def key(ui, name):
    value = {'Tab':'\ue004','Enter':'\ue007','Escape':'\ue00c'}[name]
    ui.call('/actions','POST',{'actions':[{'type':'key','id':'native-keyboard','actions':[{'type':'keyDown','value':value},{'type':'keyUp','value':value}]}]})
def tab(ui, selector):
    for i in range(120):
        if read(ui,'return document.activeElement.matches(arguments[0]);',[selector]): return
        key(ui,'Tab')
    raise q.Failure('Native Tab did not reach '+selector)
def check(ui, selector, expected, description):
    q.wait(description, lambda: read(ui,selector)==expected, 6)
def test_variant(variant, root):
    rec = {'stages':{}}
    result['variants'][variant]=rec
    package = base / (variant+'.deb')
    rec['package_sha256']=q.sha(package)
    extracted = root / 'extracted'
    extracted.mkdir()
    q.checked(['dpkg-deb','-x',package,extracted],seconds=60)
    binary = extracted / 'usr/bin/swarmcraft-desktop'
    rec['files']={n:q.sha(binary if n=='desktop' else q.sidecar(binary,n)) for n in ['desktop',*q.SIDECARS]}
    profile=root / 'profile'
    profile.mkdir(mode=0o700)
    name='LongWorldName'*9
    created=q.checked([q.sidecar(binary,'swarmcraft'),'--data-dir',profile/'data','world','create','--name',name,'--minecraft','26.1.2','--fabric-loader','0.19.3','--compatibility','vanilla-fabric','--visibility','private'],env=q.profile_env(profile)).decode()
    world=re.search(q.WORLD_PATTERN,created).group()
    checkpoint=q.verified_checkpoint(binary,profile,world)
    q.require(q.consent(binary,profile,world)['consented'] is False,'fixture opted in')
    with q.WebDriver(binary, profile, args.driver.resolve(), args.native_driver.resolve()) as ui:
        q.ready(ui,False)
        rec['engine']=ui.capabilities
        rec['stages']['boot']=True
        for width,height in [(980,760),(720,560)]:
            stage={}
            rec['stages'][f'{width}x{height}']=stage
            ui.call('/window/rect','POST',{'width':width,'height':height,'x':0,'y':0})
            q.wait('exact native viewport',lambda:read(ui,'return [innerWidth,innerHeight]')==[width,height],8)
            tab(ui,'#skipToCurrentView'); key(ui,'Enter')
            check(ui,'return document.activeElement.textContent.trim();','Your worlds','skip moved to view')
            tab(ui,'.world-row'); key(ui,'Enter')
            q.wait('selected long name',lambda:read(ui,'return document.getElementById("selectedName").textContent')==name,8)
            q.require(read(ui,'return document.getElementById("selectedWorldId").textContent')==world,'world ID not preserved')
            tab(ui,'#inviteWorld'); key(ui,'Enter')
            check(ui,'return document.activeElement.id','inviteMinutes','initial invite focus')
            obs=read(ui,'''const d=document.getElementById("inviteDialog"), r=d.getBoundingClientRect(); return {open:d.open, name:document.getElementById(d.getAttribute("aria-labelledby")).textContent.includes("LongWorldName"), description:!!document.getElementById(d.getAttribute("aria-describedby")), bounds:r.left>=0&&r.top>=0&&r.right<=innerWidth&&r.bottom<=innerHeight, noOverflow:d.scrollWidth<=d.clientWidth+2, live:document.querySelector(".topbar-status").getAttribute("aria-live")==="polite", closeWidth:document.getElementById("closeInvite").getBoundingClientRect().width};''')
            q.require(all(obs[k] for k in ['open','name','description','bounds','noOverflow','live']) and obs['closeWidth']>=28,'invite native semantics/layout failed: '+str(obs))
            ui.screenshot(artifacts/f'native-{variant}-{width}x{height}-invite.png')
            key(ui,'Escape'); check(ui,'return document.getElementById("inviteDialog").open',False,'escape closed invite')
            check(ui,'return document.activeElement.id','inviteWorld','invite focus restored')
            tab(ui,'#advancedWorldTools > summary')
            if not read(ui,'return document.getElementById("advancedWorldTools").open'): key(ui,'Enter')
            tab(ui,'#sleepWorld'); key(ui,'Enter')
            check(ui,'return document.activeElement.id','cancelSleep','safe stop focus')
            key(ui,'Escape'); check(ui,'return document.activeElement.id','sleepWorld','stop focus restored')
            stage.update({'keyboard_only':True,'long_name_and_id':True,'dialog_semantics':True,'layout':True,'escape_and_focus':True,'safe_stop_cancel':True})
        rec['stages']['native_dialog_accessibility']=True
    q.require(q.verified_checkpoint(binary,profile,world)==checkpoint,'checkpoint changed')
    q.require(q.consent(binary,profile,world)['consented'] is False,'hosting changed')
    rec['stages']['checkpoint_and_consent_unchanged']=True
    print(variant,'PASS',flush=True)
with tempfile.TemporaryDirectory(prefix='swarmcraft-a11y-native-') as tmp:
    root=Path(tmp); os.chmod(root,0o700)
    display=next(i for i in range(111,160) if not Path(f'/tmp/.X{i}-lock').exists() and not Path(f'/tmp/.X11-unix/X{i}').exists())
    auth=root/'Xauthority'; auth.touch(mode=0o600)
    q.checked(['xauth','-f',auth,'add',f':{display}','MIT-MAGIC-COOKIE-1',os.urandom(16).hex()])
    for directory in ['home','runtime','config','cache','local-share']:
        (root/directory).mkdir(mode=0o700)
    os.environ.update({'DISPLAY':f':{display}','XAUTHORITY':str(auth),'HOME':str(root/'home'),'XDG_RUNTIME_DIR':str(root/'runtime'),'XDG_CONFIG_HOME':str(root/'config'),'XDG_CACHE_HOME':str(root/'cache'),'XDG_DATA_HOME':str(root/'local-share'),'GDK_BACKEND':'x11','GSETTINGS_BACKEND':'memory','WEBKIT_DISABLE_DMABUF_RENDERER':'1'})
    xvfb=subprocess.Popen([str(args.xvfb.resolve()),f':{display}','-screen','0','1440x1000x24','-nolisten','tcp','-auth',str(auth)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
    try:
        q.wait('private Xvfb',lambda:subprocess.run(['xdpyinfo'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0,12)
        for v in ['shipping','candidate']:
            dest=root/v; dest.mkdir(mode=0o700)
            try: test_variant(v,dest)
            except Exception as e:
                result['variants'].setdefault(v,{})['error']=str(e)
                print(v,'FAILED',str(e),flush=True)
        result['limitations']=['No OS screen-reader speech verification, native OS scale testing, Windows/macOS, actual runtime install or gameplay.']
        (artifacts/'native-evidence.json').write_text(json.dumps(result,indent=2)+'\n')
    finally:
        q.stop_owned_tree(xvfb)
sys.exit(0 if all('error' not in v for v in result['variants'].values()) and len(result['variants'])==2 else 1)
