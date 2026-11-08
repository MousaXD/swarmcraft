<dialog id="inviteDialog" class="dialog">
    <form id="inviteForm" class="dialog-card" method="dialog">
      <div class="dialog-heading">
        <div>
          <h2>Invite to <span id="inviteWorldName">world</span></h2>
          <p>Create a signed invite. The recipient still joins through canonical membership.</p>
        </div>
        <button id="closeInvite" class="icon-button" value="cancel" type="submit" aria-label="Close invite dialog">×</button>
      </div>
      <div class="field">
        <label for="inviteMinutes">Invite expires in</label>
        <select id="inviteMinutes">
          <option value="30">30 minutes</option>
          <option value="60" selected>1 hour</option>
          <option value="360">6 hours</option>
          <option value="1440">24 hours</option>
        </select>
      </div>
      <details class="form-details">
        <summary>Advanced bootstrap addresses</summary>
        <div class="field details-fields">
          <label for="bootstrapAddrs">Bootstrap addresses</label>
          <textarea id="bootstrapAddrs" class="machine-input" rows="3" placeholder="/ip4/…/p2p/…"></textarea>
        </div>
      </details>
      <div id="inviteError" class="form-error" role="alert" hidden></div>
      <div id="inviteResultWrap" class="invite-result" hidden>
        <label for="inviteResult">Signed invite</label>
        <textarea id="inviteResult" rows="5" readonly spellcheck="false"></textarea>
        <button id="copyInvite" class="button button-secondary" type="button">Copy invite</button>
      </div>
      <div class="dialog-actions">
        <button id="cancelInvite" class="button button-secondary" value="cancel" type="submit">Done</button>
        <button id="createInvite" class="button button-primary world-required" type="button">Create invite</button>
      </div>
    </form>
  </dialog>

<dialog id="sleepDialog" class="dialog">
    <form class="dialog-card" method="dialog">
      <div>
        <h2>Stop this world?</h2>
        <p>SwarmCraft will stop the local Minecraft runtime. This does not itself create a durable sleeping migration state. Background replica storage can continue separately.</p>
      </div>
      <div class="dialog-actions">
        <button class="button button-secondary" value="cancel" type="submit">Cancel</button>
        <button id="confirmSleep" class="button button-warning" value="default" type="button">Stop world</button>
      </div>
    </form>
  </dialog>

<dialog id="leaveDialog" class="dialog">
    <form method="dialog" class="dialog-card">
      <div>
        <h2>Leave this world?</h2>
        <p id="leaveDialogText">This stages a signed leave request for the selected world.</p>
      </div>
      <div class="dialog-actions">
        <button id="cancelLeave" class="button button-secondary" value="cancel" type="submit">Cancel</button>
        <button id="confirmLeave" class="button button-danger" value="default" type="button">Leave world</button>
      </div>
    </form>
  </dialog>
