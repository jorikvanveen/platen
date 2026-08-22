<script lang="ts">
    import { API_URL } from "$lib/constants";
    import type { ImportSummary } from "$lib/dto/ImportSummary";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();

    let importing = $state(false);
    let summary = $state<ImportSummary | null>(null);
    let importError = $state<string | null>(null);

    async function importFromJellyfin() {
        importing = true;
        importError = null;
        try {
            const resp = await fetch(`${API_URL}/jellyfin/import`, { method: "POST" });
            if (!resp.ok) {
                importError = `Import failed (${resp.status})`;
                return;
            }
            summary = await resp.json() as ImportSummary;
        } catch (e) {
            importError = `Import failed: ${e}`;
        } finally {
            importing = false;
        }
    }
</script>

<h1>Artists</h1>
<a href="/artist/add">Add artist</a><br/><br/>
<button onclick={importFromJellyfin} disabled={importing}>
    {importing ? "Importing..." : "Import from Jellyfin"}
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
    <a href={"/artist/" + artist.musicbrainz_id}>{artist.name}</a><br/>
{/each}
