<script lang="ts">
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
</script>

<h1>Artists</h1>
<a href="/artist/add">Add artist</a><br/><br/>

{#each data.artists as artist}
    <a href={"/artist/" + artist.id}>{artist.name}</a><br/>
{/each}
