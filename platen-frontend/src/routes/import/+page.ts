import type { PageLoad } from "./$types";
import { getCatalogScan } from "$lib/catalogScan";

export const load: PageLoad = async ({ fetch }) => ({
	scan: await getCatalogScan(fetch),
});
