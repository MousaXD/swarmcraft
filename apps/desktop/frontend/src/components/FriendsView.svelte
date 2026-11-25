<section id="friendsPanel" class="view task-view" data-view-panel="friends" aria-labelledby="friendsTitle" hidden>
          <div class="task-shell friends-shell">
            <div class="task-header friends-header">
              <div><h2 id="friendsTitle">Friends</h2><p>Manage verified device contacts. Presence is private by default and never grants world membership.</p></div>
              <button id="friendsRefresh" class="button button-secondary" type="button">Refresh</button>
            </div>
            <div id="friendsStatus" class="inline-notice" role="status" aria-live="polite" aria-atomic="true">Loading contact settings…</div>
            <section class="friends-privacy" aria-labelledby="friendsPrivacyTitle">
              <div class="friends-privacy-heading">
                <div><h3 id="friendsPrivacyTitle">Your presence</h3><p id="friendsModeText" class="field-help">Settings not verified.</p></div>
                <button id="friendsModeToggle" class="button button-secondary" type="button" disabled>Enable sharing</button>
              </div>
              <p class="field-help">Only explicitly allowed contacts can receive a fresh signed online observation. Hide withdraws access, not membership. Past observations may remain until expiry. Checking a contact sends an authenticated request, even when hidden.</p>
              <div class="friends-privacy-heading">
                <span id="friendsServiceStatus" class="field-help">Presence service not verified.</span>
                <button id="friendsServiceToggle" class="button button-secondary button-small" type="button" disabled>Start service</button>
              </div>
              <p class="field-help">Start the presence-only service to answer allowed contacts. It does not list worlds. Reachability depends on configured directory and network; hiding is not network anonymity.</p>
            </section>
            <section class="friends-list-section" aria-labelledby="friendsContactsTitle">
              <div class="friends-privacy-heading"><h3 id="friendsContactsTitle">Contacts</h3><button id="friendsInvite" class="button button-secondary button-small" type="button">Create world invite</button></div>
              <p class="field-help">Choose a local world and create a signed bearer invite to share manually. It is not bound to a contact, and membership still requires approval.</p>
              <ul id="friendsContacts" class="friends-list" aria-label="Local contacts" aria-busy="false"></ul>
            </section>
            <details class="task-secondary friends-add">
              <summary>Add a contact</summary>
              <p>Get the friend's exact peer ID and public key through a trusted channel. Matching keys identify a device, not a real-world person.</p>
              <form id="friendAddForm" class="friends-add-form" novalidate>
                <div class="field"><label for="friendLabel">Local label</label><input id="friendLabel" maxlength="96" autocomplete="off" required /></div>
                <div class="field"><label for="friendPeer">Peer ID</label><input id="friendPeer" class="machine-input" maxlength="71" placeholder="scpeer:…" spellcheck="false" autocomplete="off" required /></div>
                <div class="field"><label for="friendKey">Public key (hex)</label><input id="friendKey" class="machine-input" maxlength="64" spellcheck="false" autocomplete="off" required /></div>
                <div class="form-actions"><button id="friendAdd" class="button button-primary" type="submit" disabled>Add contact</button></div>
              </form>
            </details>
            <details class="task-secondary friends-identity">
              <summary>Your contact identity</summary>
              <p>Share these public values directly with someone you trust. Never share your private identity key or world invites in a public directory.</p>
              <div class="field"><label for="friendOwnPeer">Your peer ID</label><input id="friendOwnPeer" class="machine-input" readonly /></div>
              <div class="field"><label for="friendOwnKey">Your public key</label><input id="friendOwnKey" class="machine-input" readonly /></div>
            </details>
          </div>
        </section>
