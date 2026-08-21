<script lang="ts">
    import { API_URL } from "$lib/constants";
    import Pagination from "$lib/components/Pagination.svelte";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props()

    let added_ids: string[] = $state([]);
    async function addReleaseGroup(id: string) {
      let resp = await fetch(`${API_URL}/artist/${data.artist.musicbrainz_id}/release-group/${id}`, {
        method: "POST"
      })

      if (resp.ok) {
        added_ids.push(id);
      }
    }
</script>

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
        {#if data.release_groups.length === 0}
            <tr>
                <td colspan="4">No results</td>
            </tr>
        {:else}
            {#each data.release_groups as release_group}
                <tr>
                    <td>{release_group.title}</td>
                    <td>{release_group.first_release_date}</td>
                    <td>{release_group.primary_type}</td>
                    <td>
                        {#if !added_ids.includes(release_group.id)}
                            <button onclick={() => addReleaseGroup(release_group.id)}>Add</button>
                        {/if}
                    </td>
                </tr>
            {/each}
        {/if}
    </tbody>
</table>
<br/>
<Pagination
    page={data.page}
    pages={data.pages}
    hrefForPage={(n) => `/artist/${data.artist.musicbrainz_id}/add-release-group?page=${n}`}
/>
