<script lang="ts">
    import { goto } from "$app/navigation";
    import { API_URL } from "$lib/constants";
    import type { TidalAlbum } from "$lib/dto/TidalAlbum";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();

    // query is intentionally a local, mutable copy of the URL query param
    // svelte-ignore state_referenced_locally
    let query = $state(data.query);

    // Per-artist expand state and lazy-loaded album cache.
    let expanded: Record<string, boolean> = $state({});
    let loading: Record<string, boolean> = $state({});
    let albumsCache: Record<string, TidalAlbum[]> = $state({});

    function submit() {
      goto(`/artist/add?q=${encodeURIComponent(query)}`);
    }

    async function addArtist(id: string) {
      const resp = await fetch(`${API_URL}/artists/${id}`, {
        method: "POST",
      })

      if (resp.ok) {
        goto(`/artist/${id}`)
      }
    }

    async function toggleExpand(id: string) {
      if (expanded[id]) {
        expanded[id] = false;
        return;
      }

      expanded[id] = true;

      if (albumsCache[id] === undefined) {
        loading[id] = true;
        try {
          const resp = await fetch(`${API_URL}/tidal/artists/${id}`);
          if (resp.ok) {
            const all: TidalAlbum[] = await resp.json();
            albumsCache[id] = all
              .slice()
              .sort((a, b) => b.popularity - a.popularity)
              .slice(0, 5);
          } else {
            albumsCache[id] = [];
          }
        } catch {
          albumsCache[id] = [];
        } finally {
          loading[id] = false;
        }
      }
    }
</script>

<h1>Add artist</h1>
<form onsubmit={(e) => { e.preventDefault(); submit(); }}>
    <input id="search" type="text" bind:value={query}>
    <button type="submit">Search</button>
</form>
<br/>
<br/>
<br/>

{#if data.artists !== null}
    {#if data.artists.length === 0}
        <span>No results</span>
    {:else}
        <table>
            <thead>
                <tr>
                    <th>Name</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
            {#each data.artists as result (result.id)}
                <tr>
                    <!-- svelte-ignore a11y_invalid_attribute -->
                    <td><a onclick={() => addArtist(result.id)} href="#">{result.name}</a></td>
                    <td>
                        <button onclick={() => toggleExpand(result.id)}>
                            {expanded[result.id] ? "−" : "+"}
                        </button>
                    </td>
                </tr>
                {#if expanded[result.id]}
                    <tr>
                        <td colspan="2">
                            {#if loading[result.id]}
                                <span>Loading albums…</span>
                            {:else if albumsCache[result.id]?.length === 0}
                                <span>No albums</span>
                            {:else}
                                <table>
                                    <thead>
                                        <tr>
                                            <th>Title</th>
                                            <th>Date</th>
                                            <th>Type</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {#each albumsCache[result.id] as album (album.id)}
                                            <tr>
                                                <td>{album.title}</td>
                                                <td>{album.release_date || ""}</td>
                                                <td>{album.type || ""}</td>
                                            </tr>
                                        {/each}
                                    </tbody>
                                </table>
                            {/if}
                        </td>
                    </tr>
                {/if}
            {/each}
            </tbody>
        </table>
    {/if}
{/if}
