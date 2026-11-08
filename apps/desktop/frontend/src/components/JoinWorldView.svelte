<section id="joinPanel" class="view task-view" data-view-panel="join" aria-labelledby="joinTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="joinTitle">Join with an invite</h2>
                <p>Paste the complete signed invite from a world member. Discovery alone never grants membership.</p>
              </div>
            </div>

            <form id="joinForm" class="form-panel" novalidate>
              <div class="field field-wide">
                <label for="joinInvite">Invite</label>
                <textarea id="joinInvite" rows="7" placeholder="scinvite:…" required spellcheck="false"></textarea>
              </div>
              <div id="joinError" class="form-error field-wide" role="alert" hidden></div>
              <div class="form-actions field-wide">
                <button id="cancelJoin" class="button button-secondary" type="button">Cancel</button>
                <button id="joinWorld" class="button button-primary" type="submit">Join world</button>
              </div>
            </form>

            <details class="task-secondary">
              <summary>Have only a World ID?</summary>
              <p>A World ID can check discovery for public or unlisted worlds, but you still need accepted membership before joining a private world.</p>
              <div class="inline-field">
                <label for="joinWorldId">World ID</label>
                <input id="joinWorldId" class="machine-input" placeholder="scworld:…" />
                <button id="joinWorldIdButton" class="button button-secondary" type="button">Check</button>
              </div>
              <div id="joinWorldIdNotice" class="inline-notice" role="status" hidden></div>
            </details>
          </div>
        </section>
