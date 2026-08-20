<script lang="ts">
    import { API_URL } from "$lib/constants";
    import type { PageProps } from "./$types";

    const { data }: PageProps = $props();

    let download_state: { [key: string]: "downloading" | "done" | undefined } = $state({});

    async function download(id: string) {
      download_state[id] = "downloading";
      const resp = await fetch(`${API_URL}/artist/${data.artist.musicbrainz_id}/release-group/${id}/download`, {
        method: "POST"
      });

      if (resp.ok) {
        download_state[id] = "done";
      } else {
        download_state[id] = undefined;
      }
    }
</script>
<h1>{data.artist.name}</h1>
<a href={"/artist/" + data.artist.musicbrainz_id + "/add-release-group/0"}>Add release group</a><br/><br/>

<table>
    <thead>
        <tr>
            <th>Name</th>
            <th>Downloaded</th>
            <th></th>
        </tr>
    </thead>
    <tbody>
        {#each data.release_groups as release_group}
            <tr>
                <td>{release_group.title}</td>
                <td>
                    {#if release_group.downloaded || download_state[release_group.musicbrainz_id] == "done"}
                        Downloaded
                    {:else}
                        <button disabled={download_state[release_group.musicbrainz_id] == "downloading"} onclick={() => download(release_group.musicbrainz_id)}>Download</button>
                    {/if}
                </td>
                <td></td>
            </tr>
        {/each}
    </tbody>
</table>