<aside class="sidebar" aria-label="Primary navigation">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">S</span>
        <div class="brand-copy">
          <strong>SwarmCraft</strong>
          <span>Minecraft worlds</span>
        </div>
      </div>

      <nav class="nav-stack" aria-label="Main">
        <button id="navWorlds" class="nav-item is-active" type="button" data-view="worlds" aria-current="page">
          <span class="nav-glyph" aria-hidden="true">▦</span><span>Worlds</span>
        </button>
        <button id="openCreate" class="nav-item" type="button" data-view="create">
          <span class="nav-glyph" aria-hidden="true">＋</span><span>Create</span>
        </button>
        <button id="openImport" class="nav-item" type="button" data-view="import">
          <span class="nav-glyph" aria-hidden="true">⇩</span><span>Import</span>
        </button>
        <button id="openJoin" class="nav-item" type="button" data-view="join">
          <span class="nav-glyph" aria-hidden="true">↳</span><span>Join</span>
        </button>
        <button id="navActivity" class="nav-item" type="button" data-view="activity">
          <span class="nav-glyph" aria-hidden="true">≡</span><span>Activity</span>
        </button>
        <button id="navDiagnostics" class="nav-item" type="button" data-view="diagnostics">
          <span class="nav-glyph" aria-hidden="true">⌁</span><span>Diagnostics</span>
        </button>
      </nav>

      <div class="sidebar-node">
        <div class="node-line">
          <span id="nodeStateDot" class="state-dot neutral" aria-hidden="true"></span>
          <div class="node-copy">
            <span class="sidebar-label">This device</span>
            <span id="nodeBadge" class="node-state">Checking…</span>
          </div>
        </div>
        <button id="sidebarInit" class="button button-subtle button-small" type="button">Set up device</button>
      </div>
    </aside>
