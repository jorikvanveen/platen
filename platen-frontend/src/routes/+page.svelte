<script lang="ts">
    import { invalidateAll } from "$app/navigation";
    import { API_URL } from "$lib/constants";
    import type { ImportStatus } from "$lib/dto/ImportStatus";
    import type { ImportSummary } from "$lib/dto/ImportSummary";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();

    let importing = $state(false);
    let summary = $state<ImportSummary | null>(null);
    let importError = $state<string | null>(null);
    let serverRunning = $state(false);
    // Previous poll's running flag, used to detect running -> idle transitions.
    // Plain (non-reactive): only read/written inside the poll callback.
    let prevServerRunning = false;

    async function importFromJellyfin() {
        importing = true;
        importError = null;
        try {
            const resp = await fetch(`${API_URL}/jellyfin/import`, { method: "POST" });
            if (!resp.ok) {
                // A 409 means an import is already running on the server. Leave
                // the existing summary alone; the poll catches up serverRunning.
                if (resp.status === 409) {
                    importError = "An import is already running";
                } else {
                    importError = `Import failed (${resp.status})`;
                }
                return;
            }
            summary = await resp.json() as ImportSummary;
            await invalidateAll();
        } catch (e) {
            importError = `Import failed: ${e}`;
        } finally {
            importing = false;
        }
    }

    // Poll the server-side import state every 2s while the page is mounted.
    // The returned cleanup clears the interval on unmount, so navigating away
    // stops the status requests (no leaked timers).
    $effect(() => {
        const interval = setInterval(async () => {
            try {
                const resp = await fetch(`${API_URL}/jellyfin/import/status`);
                if (!resp.ok) return;
                const status = await resp.json() as ImportStatus;
                const nowRunning = status.state === "running";
                // running -> idle with a fresh summary: surface it and refresh
                // the artist list via the +page.ts load function.
                if (prevServerRunning && !nowRunning && status.last_summary) {
                    summary = status.last_summary;
                    await invalidateAll();
                }
                prevServerRunning = nowRunning;
                serverRunning = nowRunning;
            } catch {
                // transient network error; the next tick retries
            }
        }, 2000);

        return () => clearInterval(interval);
    });
</script>

<h1>Artists</h1>
<a href="/artist/add">Add artist</a><br/><br/>
<button onclick={importFromJellyfin} disabled={importing || serverRunning}>
    {importing
        ? "Importing..."
        : serverRunning
            ? "Import running (started elsewhere)"
            : "Import from Jellyfin"}
</button>
<br/><br/>

{#if importError}
    <p>{importError}</p>
{/if}

{#if summary}
    <h2>Import summary</h2>
    <p>Scanned: {summary.total_scanned}</p>
    <p>Created: {summary.created}</p>
    <p>Linked: {summary.linked}</p>
    <p>Skipped: {summary.skipped}</p>
    <p>Failed: {summary.failed}</p>

    {#if summary.failures.length > 0}
        <h3>Failures</h3>
        <ul>
            {#each summary.failures as failure}
                <li>{failure.name}: {failure.reason}</li>
            {/each}
        </ul>
    {/if}
{/if}

<br/>
{#each data.artists as artist}
    <a href={"/artist/" + artist.id}>{artist.name}</a><br/>
{/each}
