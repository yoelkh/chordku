// Standalone setlist viewer: one HTML file, no server. The setlist data is embedded by the
// ChordKu server when the file is downloaded (`GET /api/setlists/{id}/share.html`).
import { mount } from "svelte";
import Viewer from "./Viewer.svelte";
import "../app.css";

mount(Viewer, { target: document.getElementById("app")! });
