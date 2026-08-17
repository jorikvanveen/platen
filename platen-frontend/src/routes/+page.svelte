<script lang="ts">
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
    let a = data.artists;
</script>
<h1>Artists</h1>
<a href="/artist/add">Add artist</a><br/><br/>
{#each data.artists as artist}
    <a href={"/artist/" + artist.musicbrainz_id}>{artist.name}</a><br/>
{/each}
