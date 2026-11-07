function clean(value) {
  return String(value ?? '').trim();
}

function opaqueSessionFromDestination(destination) {
  const value = clean(destination);
  const match = value.match(/^(desktop-[A-Za-z0-9-]{1,120})[\\/]/);
  if (!match) return '';
  const session = match[1];
  return session.length <= 128 ? session : '';
}

export function adaptProviderInvocation(command, args = {}) {
  if (command === 'modrinth_download') {
    const request = args?.request;
    const stagingSession = opaqueSessionFromDestination(request?.destination_dir ?? request?.destinationDir);
    if (stagingSession && request?.locator) {
      return {
        command,
        args: {
          locator: request.locator,
          stagingSession,
          maxBytes: request.max_bytes ?? request.maxBytes ?? null,
        },
      };
    }
  }

  if (command === 'curseforge_download') {
    const stagingSession = opaqueSessionFromDestination(args?.destination);
    if (stagingSession && args?.fileId != null) {
      return {
        command,
        args: {
          fileId: args.fileId,
          stagingSession,
        },
      };
    }
  }

  return { command, args };
}

// The native Tauri 2 bridge exposes core.invoke as a READ-ONLY property in
// actual packaged WebViews. Never monkey-patch it during module evaluation:
// doing so throws before app.js can initialize and leaves Loading/Checking
// forever. Instead wrap each app-owned call site explicitly.
export function providerContractInvoke(core) {
  const nativeInvoke = core?.invoke;
  if (typeof nativeInvoke !== 'function') return null;
  return (command, args = {}) => {
    const adapted = adaptProviderInvocation(command, args);
    return nativeInvoke.call(core, adapted.command, adapted.args);
  };
}

// Legacy explicit opt-in helper for mutable test/extension contexts. Production
// uses providerContractInvoke and never calls this function implicitly.
export function installProviderContractBridge(target = globalThis.window) {
  const core = target?.__TAURI__?.core;
  if (typeof core?.invoke !== 'function' || core.invoke.__swarmcraftProviderContractBridge) return false;
  const descriptor = Object.getOwnPropertyDescriptor(core, 'invoke');
  if (descriptor && descriptor.writable === false && typeof descriptor.set !== 'function') return false;
  const bridgedInvoke = providerContractInvoke(core);
  Object.defineProperty(bridgedInvoke, '__swarmcraftProviderContractBridge', { value: true });
  try {
    core.invoke = bridgedInvoke;
  } catch (_) {
    return false;
  }
  return core.invoke === bridgedInvoke;
}
