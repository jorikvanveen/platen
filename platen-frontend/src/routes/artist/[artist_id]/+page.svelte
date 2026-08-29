<script lang="ts">
    import { API_URL } from "$lib/constants";
    import type { PageProps } from "./$types";

    const { data }: PageProps = $props();

    let download_state: { [key: string]: "downloading" | "done" | undefined } = $state({});

    async function download(id: string) {
      download_state[id] = "downloading";
      const resp = await fetch(`${API_URL}/albums/${id}/download`, {
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
<a href={"/artist/" + data.artist.id + "/add-album"}>Add album</a><br/><br/>

<table>
    <thead>
        <tr>
            <th>Name</th>
            <th>Artists</th>
            <th>Downloaded</th>
            <th></th>
        </tr>
    </thead>
    <tbody>
        {#each data.albums as album}
            <tr>
                <td>{album.title}</td>
                <td>{album.artists.map((a) => a.name).join(", ")}</td>
                <td>
                    {#if download_state[album.id] == "done"}
                        Downloaded
                    {:else}
                        <button disabled={download_state[album.id] == "downloading"} onclick={() => download(album.id)}>Download</button>
                    {/if}
                </td>
                <td></td>
            </tr>
        {/each}
    </tbody>
</table>
