<script lang="ts">
  import Topbar from './Topbar.svelte';
  import WorldsView from './WorldsView.svelte';
  import CreateWorldView from './CreateWorldView.svelte';
  import ImportWorldView from './ImportWorldView.svelte';
  import JoinWorldView from './JoinWorldView.svelte';
  import ActivityView from './ActivityView.svelte';
  import DiagnosticsView from './DiagnosticsView.svelte';
</script>

<main class="workspace">
  <Topbar />
  <div class="view-stack">
    <WorldsView />
    <CreateWorldView />
    <ImportWorldView />
    <JoinWorldView />
    <ActivityView />
    <DiagnosticsView />
  </div>
</main>

