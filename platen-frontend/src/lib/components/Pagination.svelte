<script lang="ts">
    let { page, pages, hrefForPage }: {
        page: number;
        pages: number;
        hrefForPage: (n: number) => string;
    } = $props();
</script>

{#if pages > 1}
    <div>
        <a href={hrefForPage(page - 1)} aria-disabled={page <= 0} class={page <= 0 ? "disabled" : ""}>Prev</a>
        <span>Page {page + 1} of {pages}</span>
        <a href={hrefForPage(page + 1)} aria-disabled={page >= pages - 1} class={page >= pages - 1 ? "disabled" : ""}>Next</a>
    </div>
{/if}

<style>
    a.disabled {
        pointer-events: none;
        opacity: 0.5;
    }
</style>
