<script lang="ts">
    import { API_URL } from "$lib/constants";
    import { groupAlbums } from "$lib/albumGroups";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props()

    let added_ids: string[] = $state([]);
    async function addAlbum(id: string) {
      let resp = await fetch(`${API_URL}/albums/${id}`, {
        method: "POST"
      })

      if (resp.ok) {
        added_ids.push(id);
      }
    }

    function isAdded(id: string): boolean {
      return data.existing_album_ids.includes(id) || added_ids.includes(id);
    }

    const groups = groupAlbums(data.albums);
</script>

<h1>{data.artist.name}</h1>
{#if data.albums.length === 0}
    <table>
        <thead>
            <tr>
                <th>Name</th>
                <th>Date</th>
                <th></th>
            </tr>
        </thead>
        <tbody>
            <tr>
                <td colspan="3">No results</td>
            </tr>
        </tbody>
    </table>
{:else}
    {#each groups as group}
        <h2>{group.label}</h2>
        <table>
            <thead>
                <tr>
                    <th>Name</th>
                    <th>Date</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {#each group.albums as album}
                    <tr>
                        <td>{album.title}</td>
                        <td>{album.release_date || ""}</td>
                        <td>
                            {#if !isAdded(album.id)}
                                <button onclick={() => addAlbum(album.id)}>Add</button>
                            {/if}
                        </td>
                    </tr>
                {/each}
            </tbody>
        </table>
    {/each}
{/if}
