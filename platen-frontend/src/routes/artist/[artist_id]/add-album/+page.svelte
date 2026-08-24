<script lang="ts">
    import { API_URL } from "$lib/constants";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props()

    let added_ids: string[] = $state([]);
    async function addAlbum(id: string) {
      let resp = await fetch(`${API_URL}/artists/${data.artist.id}/albums/${id}`, {
        method: "POST"
      })

      if (resp.ok) {
        added_ids.push(id);
      }
    }

    function isAdded(id: string): boolean {
      return data.existing_album_ids.includes(id) || added_ids.includes(id);
    }
</script>

{@debug data}
<h1>{data.artist.name}</h1>
<table>
    <thead>
        <tr>
            <th>Name</th>
            <th>Date</th>
            <th>Type</th>
            <th></th>
        </tr>
    </thead>
    <tbody>
        {#if data.albums.length === 0}
            <tr>
                <td colspan="4">No results</td>
            </tr>
        {:else}
            {#each data.albums as album}
                <tr>
                    <td>{album.title}</td>
                    <td>{album.release_date || ""}</td>
                    <td>{album.album_type || ""}</td>
                    <td>
                        {#if !isAdded(album.id)}
                            <button onclick={() => addAlbum(album.id)}>Add</button>
                        {/if}
                    </td>
                </tr>
            {/each}
        {/if}
    </tbody>
</table>
