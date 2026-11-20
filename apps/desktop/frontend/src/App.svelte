<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import Workspace from './components/Workspace.svelte';
  import Dialogs from './components/Dialogs.svelte';
  import { activityBridge } from './lib/activity';

  // Compatibility phase: mount ALL original elements before attaching legacy
  // Rust-backed event handlers. This does not constitute reactive parity.
  onMount(() => {
    const previous = window.__swarmcraftSvelteActivity;
    const previousCreateOwner = window.__swarmcraftSvelteCreateOwned;
    const previousJoinOwner = window.__swarmcraftSvelteJoinOwned;
    const previousImportOwner = window.__swarmcraftSvelteImportOwned;
    const previousInviteOwner = window.__swarmcraftSvelteInviteOwned;
    const previousDiscoveryOwner = window.__swarmcraftSvelteDiscoveryOwned;
    const previousLeaveOwner = window.__swarmcraftSvelteLeaveOwned;
    const previousIdentityOwner = window.__swarmcraftSvelteIdentityOwned;
    window.__swarmcraftSvelteLeaveOwned = true;
    window.__swarmcraftSvelteIdentityOwned = true;
    window.__swarmcraftSvelteDiscoveryOwned = true;
    window.__swarmcraftSvelteInviteOwned = true;
    window.__swarmcraftSvelteImportOwned = true;
    window.__swarmcraftSvelteJoinOwned = true;
    window.__swarmcraftSvelteCreateOwned = true;
    window.__swarmcraftSvelteActivity = activityBridge;
    void import('../../src/app.js').catch(() => {
      window.__swarmcraftStartup?.fail(
        'module-load',
        'The desktop controller failed to load. Check the installation and retry.',
      );
    });
    return () => {
      window.__swarmcraftSvelteCreateOwned = previousCreateOwner;
      window.__swarmcraftSvelteJoinOwned = previousJoinOwner;
      window.__swarmcraftSvelteImportOwned = previousImportOwner;
      window.__swarmcraftSvelteInviteOwned = previousInviteOwner;
      window.__swarmcraftSvelteDiscoveryOwned = previousDiscoveryOwner;
      window.__swarmcraftSvelteLeaveOwned = previousLeaveOwner;
      window.__swarmcraftSvelteIdentityOwned = previousIdentityOwner;
      if (window.__swarmcraftSvelteActivity === activityBridge) {
        window.__swarmcraftSvelteActivity = previous;
      }
    };
  });
</script>

<div class="app-shell">
  <Sidebar />
  <Workspace />
</div>
<Dialogs />
