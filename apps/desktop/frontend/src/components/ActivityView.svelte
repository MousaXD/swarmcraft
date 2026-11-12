<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { activityText, clearActivity } from '../lib/activity';

  let output: HTMLPreElement | undefined;

  onMount(() => {
    const unsubscribe = activityText.subscribe(async () => {
      await tick();
      if (output) output.scrollTop = output.scrollHeight;
    });
    return unsubscribe;
  });

  function clear(event: MouseEvent) {
    // Prevent the old JS compatibility listener from clearing the DOM only.
    event.stopImmediatePropagation();
    clearActivity();
  }
</script>

<section id="activityView" class="view" data-view-panel="activity" aria-labelledby="activityTitle" hidden>
  <div class="content-shell activity-shell">
    <div class="pane-head standalone-head">
      <div>
        <h2 id="activityTitle">Activity</h2>
        <p class="meta-text">Recent command results and detailed errors from this session.</p>
      </div>
      <button id="clearActivity" class="button button-subtle button-small" type="button" onclick={clear}>Clear</button>
    </div>
    <pre id="output" bind:this={output} class="activity-console" role="textbox" aria-readonly="true" aria-multiline="true" tabindex="0" aria-label="Activity log">{$activityText}</pre>
  </div>
</section>
