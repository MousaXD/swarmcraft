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

export function installProviderContractBridge(target = globalThis.window) {
  const core = target?.__TAURI__?.core;
  const nativeInvoke = core?.invoke;
  if (typeof nativeInvoke !== 'function' || nativeInvoke.__swarmcraftProviderContractBridge) return false;

  const bridgedInvoke = function bridgedProviderInvoke(command, args = {}) {
    const adapted = adaptProviderInvocation(command, args);
    return nativeInvoke.call(core, adapted.command, adapted.args);
  };
  Object.defineProperty(bridgedInvoke, '__swarmcraftProviderContractBridge', { value: true });
  core.invoke = bridgedInvoke;
  return true;
}

installProviderContractBridge();
