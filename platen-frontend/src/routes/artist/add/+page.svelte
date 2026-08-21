<script lang="ts">
    import { goto } from "$app/navigation";
    import { API_URL } from "$lib/constants";
    import Pagination from "$lib/components/Pagination.svelte";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();

    let query = $state(data.query);

    function submit() {
      goto(`/artist/add?q=${encodeURIComponent(query)}&page=0`);
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

{#if data.artists !== null}
    {#if data.artists.length === 0}
        <span>No results</span>
    {:else}
        <table>
            <thead>
                <tr>
                    <th>Name</th>
                    <th>Country</th>
                    <th>Disambiguation</th>
                </tr>
            </thead>
            <tbody>
            {#each data.artists as result}
                <tr>
                    <!-- svelte-ignore a11y_invalid_attribute -->
                    <td><a onclick={() => addArtist(result.id)} href="#">{result.name}</a></td>
                    <td>{result.country || ""}</td>
                    <td>{result.disambiguation || ""}</td>
                </tr>
            {/each}
            </tbody>
        </table>
        <br/>
        <Pagination
            page={data.page}
            pages={data.pages}
            hrefForPage={(n) => `/artist/add?q=${encodeURIComponent(data.query)}&page=${n}`}
        />
    {/if}
{/if}
