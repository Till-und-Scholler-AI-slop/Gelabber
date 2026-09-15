# #53 Capture diagnostics — how to enable

**Branch:** `diag/53-capture-settings`  
**Code:** `web/src/voice/captureDiag.ts` (wired from `session.ts`)  
**No gain hack / no bitrate change / diagnostics only.**

## Enable the flag (any one)

1. **Query:** join voice with `?gelabberDiag=1` in the URL  
2. **localStorage:** in DevTools console: `localStorage.setItem('gelabberDiag','1')` then reload  
3. **Build env:** `VITE_GELABBER_DIAG=1` when building/running Vite

## What happens when enabled

After `getUserMedia` succeeds:

- `console.info('[gelabberDiag] capture settings', …)` with requested `MIC_AUDIO` keys vs `track.getSettings()`
- Stash on `window.__gelabberCaptureDiag`
- Browser download: `capture-settings-<iso>.json`

Optional A/B MediaRecorder (~20s auto-stop):

- Local mic clone → `local-<iso>.webm`
- First remote audio track on `remoteMix` → `remote-<iso>.webm`
- Manual: `window.__gelabberStartAbRecord()` / `window.__gelabberStopAbRecord()`

## Playtest save path

Move downloads into:

`output/issue-53/`

(or `output/issue-53/ab/<run-id>/` for A/B pairs). Keep UA / OS / device label from the JSON.

## Off

`localStorage.removeItem('gelabberDiag')` and drop the query/env flag.
