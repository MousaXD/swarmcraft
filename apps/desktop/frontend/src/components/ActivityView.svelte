<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { activityText, clearActivity } from '../lib/activity';

  let output: HTMLTextAreaElement | undefined;

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
        <p class="meta-text">Private local action summaries and opt-in support export.</p>
      </div>
    </div>
        <section class="support-history" aria-labelledby="supportHistoryTitle">
      <div class="section-title-row">
        <div><h3 id="supportHistoryTitle">Local world-action history</h3>
          <p class="meta-text">Optional, private to this device. Records only action categories, native return outcomes, time and build provenance; no raw command output. A local World ID is used only for filtering, never exported.</p></div>
      </div>
      <p id="supportHistoryStatus" class="inline-notice" role="status" aria-live="polite">Checking local recording preference…</p>
      <div class="support-controls">
        <div class="field"><label for="supportScope">History and export scope</label>
          <select id="supportScope"><option value="all">All world actions</option><option value="selected" disabled>Selected world only</option></select>
        </div>
        <div class="compact-actions">
          <button id="supportRecording" class="button button-secondary button-small" type="button" disabled>Enable history</button>
          <button id="supportRefresh" class="button button-subtle button-small" type="button">Refresh history</button>
          <button id="supportClear" class="button button-subtle button-small" type="button" disabled>Clear history…</button>
        </div>
      </div>
      <div id="supportClearPrompt" class="inline-notice" hidden>
        <p>Delete all recorded summaries on this device, including other worlds? This does not delete any world or raw session log.</p>
        <div class="compact-actions"><button id="supportClearConfirm" class="button button-danger button-small" type="button">Delete all history</button><button id="supportClearCancel" class="button button-secondary button-small" type="button">Cancel</button></div>
      </div>
      <p id="supportEmpty" class="meta-text">No history loaded.</p>
      <ol id="supportEvents" class="support-events" aria-label="Recorded world actions"></ol>
      <p class="helper">A successful native return is not proof that a staged membership, transfer or wake operation has committed. Latest 256 completed/interrupted Desktop requests only.</p>
    </section>
    <section class="support-export" aria-labelledby="supportExportTitle">
      <div class="section-title-row"><div><h3 id="supportExportTitle">Redacted support report</h3>
        <p class="meta-text">Nothing is sent automatically. Preview includes time, Desktop build, last reported node path class (not a game-route proof) and selected history summaries. No world/peer IDs, names, invites, credentials, raw addresses, paths or world data.</p></div></div>
      <div class="compact-actions"><button id="supportPreview" class="button button-primary button-small" type="button" disabled>Preview redacted report</button></div>
      <p id="supportExportStatus" class="inline-notice" role="status" aria-live="polite" hidden></p>
      <div id="supportPreviewPanel" class="field" hidden>
        <label for="supportPreviewText">Exact JSON to share (review before exporting)</label>
        <textarea id="supportPreviewText" class="support-json machine-input" readonly rows="8" spellcheck="false"></textarea>
        <div class="compact-actions">
          <button id="supportSave" class="button button-primary button-small" type="button" disabled>Save JSON…</button>
          <button id="supportCopy" class="button button-secondary button-small" type="button" disabled>Copy JSON</button>
          <button id="supportDiscard" class="button button-subtle button-small" type="button">Discard preview</button>
        </div>
      </div>
    </section>

    <details class="raw-activity-details form-details">
      <summary>Raw session output (not included in support report)</summary>
      <p class="meta-text">Detailed local command results may contain sensitive information. Do not share them without reviewing separately.</p>
      <div class="compact-actions"><button id="clearActivity" class="button button-subtle button-small" type="button" onclick={clear}>Clear session output</button></div>
      <textarea id="output" bind:this={output} class="activity-console"
      aria-label="Activity log" readonly value={$activityText} style:resize="none"></textarea>
    </details>
  </div>
</section>
