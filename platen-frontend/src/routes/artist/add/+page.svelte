<script lang="ts">
    import { goto } from "$app/navigation";
    import { API_URL } from "$lib/constants";

    let query = $state("");

    type Artist = {
      id: string,
      name: string,
      disambiguation: string,
      country: string
    }

    let results: Artist[] | null = $state(null);
    
    async function submit() {
      const resp = await fetch(`${API_URL}/mb/search_artist/${encodeURIComponent(query)}`)
      const results_response = await resp.json();
      results = results_response
    }

    async function addArtist(id: string) {
      const resp = await fetch(`${API_URL}/artist/${id}`, {
        method: "POST",
      })

      if (resp.ok) {
        goto(`/artist/${id}`)
      }
    }
</script>

<h1>Add artist</h1>
<input id="search" type="text" bind:value={query}>
<button onclick={submit}>Search</button>
<br/>
<br/>
<br/>

{#if results}
    <table>
        <thead>
            <tr>
                <th>Name</th>
                <th>Country</th>
                <th>Disambiguation</th>
            </tr>
        </thead>
        <tbody>
        {#each results as result}
            <tr>
                <!-- svelte-ignore a11y_invalid_attribute -->
                <td><a onclick={() => addArtist(result.id)} href="#">{result.name}</a></td>
                <td>{result.country || ""}</td>
                <td>{result.disambiguation || ""}</td>
            </tr>
        {/each}
        </tbody>
    </table>
{:else}
    <span>No results</span>
{/if}
