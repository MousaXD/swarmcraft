<section id="createPanel" class="view task-view" data-view-panel="create" aria-labelledby="createTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="createTitle">Create a world</h2>
                <p>Start a private replicated world. Advanced compatibility settings can stay at their defaults for a normal Fabric setup.</p>
              </div>
            </div>

            <form id="createForm" class="form-panel" novalidate>
              <div class="field field-wide">
                <label for="createName">World name</label>
                <input id="createName" autocomplete="off" placeholder="Friends SMP" required />
              </div>
              <div class="field field-wide">
                <label for="createVisibility">Who can find it?</label>
                <select id="createVisibility">
                  <option value="private">Invite only</option>
                  <option value="unlisted">Unlisted</option>
                  <option value="public">Public</option>
                </select>
              </div>

              <details class="form-details field-wide">
                <summary>Compatibility settings</summary>
                <div class="field-grid details-fields">
                  <div class="field">
                    <label for="createMinecraft">Minecraft version</label>
                    <input id="createMinecraft" value="26.1.2" required />
                  </div>
                  <div class="field">
                    <label for="createLoader">Fabric loader</label>
                    <input id="createLoader" value="0.19.3" required />
                  </div>
                  <div class="field field-wide">
                    <label for="createCompatibility">Compatibility profile</label>
                    <input id="createCompatibility" value="vanilla-fabric" required />
                  </div>
                </div>
                <p class="helper">These values become signed world metadata. They determine which devices are eligible to host, not which devices may store replicas.</p>
              </details>

              <div id="createError" class="form-error field-wide" role="alert" hidden></div>
              <div class="form-actions field-wide">
                <button id="cancelCreate" class="button button-secondary" type="button">Cancel</button>
                <button id="createWorld" class="button button-primary" type="submit">Create world</button>
              </div>
            </form>
          </div>
        </section>
