import { mount } from 'svelte';
import App from './App.svelte';
// Preserve the legacy visual system until every view is migrated and reviewed.
import '../../src/style.css';
import '../../src/player-experience.css';
import '../../src/runtime-wizard.css';

const target = document.getElementById('app');
if (!target) throw new Error('SwarmCraft desktop mount target is missing.');
mount(App, { target });
