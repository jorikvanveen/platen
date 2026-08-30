import { error } from "@sveltejs/kit";
import { API_URL } from "$lib/constants";
import type { Downloads } from "$lib/dto/Downloads";
import type { PageLoad } from "./$types";

export const load: PageLoad = async ({ fetch }) => {
	const response = await fetch(`${API_URL}/downloads`);
	if (!response.ok) throw error(response.status, "Could not load downloads");

	return { downloads: (await response.json()) as Downloads };
};
