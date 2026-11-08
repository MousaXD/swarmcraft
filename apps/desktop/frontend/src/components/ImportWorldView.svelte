<section id="importPanel" class="view task-view" data-view-panel="import" aria-labelledby="importTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="importTitle">Import existing world</h2>
                <p>Bring an existing Minecraft save into SwarmCraft without changing the source folder, accepting the Minecraft EULA, or importing Java/runtime binaries.</p>
              </div>
            </div>

            <form id="importForm" class="form-panel" novalidate>
              <div class="field field-wide">
                <label for="importSource">Minecraft world folder</label>
                <input id="importSource" class="machine-input" autocomplete="off" placeholder="/path/to/.minecraft/saves/My World" required />
                <p class="field-help">Paste or type the existing world folder path from your file manager. The source folder stays unchanged.</p>
              </div>
              <div class="field field-wide">
                <label for="importName">World display name</label>
                <input id="importName" autocomplete="off" placeholder="Friends SMP" required />
              </div>
              <div class="field field-wide">
                <label for="importVisibility">Who can find it?</label>
                <select id="importVisibility">
                  <option value="private">Invite only</option>
                  <option value="unlisted">Unlisted</option>
                  <option value="public">Public</option>
                </select>
              </div>
              <div class="field-grid field-wide">
                <div class="field">
                  <label for="importMinecraft">Exact Minecraft version</label>
                  <input id="importMinecraft" value="26.1.2" required />
                </div>
                <div class="field">
                  <label for="importLoader">Exact Fabric Loader version</label>
                  <input id="importLoader" value="0.19.3" required />
                </div>
              </div>
              <div class="field field-wide">
                <label for="importServerMods">Required third-party server mod JARs</label>
                <textarea id="importServerMods" class="machine-input" rows="4" placeholder="/path/to/Lithium.jar&#10;/path/to/FerriteCore.jar" spellcheck="false"></textarea>
                <p class="field-help">Enter one full local JAR path per line. The Rust backend verifies compatibility; SwarmCraft does not infer mod requirements from level.dat.</p>
              </div>
              <label class="check-row field-wide" for="importNoServerMods">
                <input id="importNoServerMods" type="checkbox" />
                <span>I confirm this world requires no third-party server mods.</span>
              </label>
              <p class="helper field-wide">Choose either required server-mod JARs or the confirmation above. Import never accepts the Minecraft EULA and never imports Java/runtime binaries.</p>
              <div id="importError" class="form-error field-wide" role="alert" hidden></div>
              <div class="form-actions field-wide">
                <button id="cancelImport" class="button button-secondary" type="button">Cancel</button>
                <button id="importWorld" class="button button-primary" type="submit">Import world</button>
              </div>
            </form>
          </div>
        </section>
