<script lang="ts">
    import { goto } from "$app/navigation";
    import { API_URL } from "$lib/constants";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();

    // query is intentionally a local, mutable copy of the URL query param
    // svelte-ignore state_referenced_locally
    let query = $state(data.query);

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
            {#each data.artists as result}
                <tr>
                    <!-- svelte-ignore a11y_invalid_attribute -->
                    <td><a onclick={() => addArtist(result.id)} href="#">{result.name}</a></td>
                    <td></td>
                </tr>
            {/each}
            </tbody>
        </table>
    {/if}
{/if}
