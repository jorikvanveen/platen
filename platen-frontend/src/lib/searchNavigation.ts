import { goto } from '$app/navigation';

export async function navigateToSearch(path: string, query: string): Promise<undefined> {
	const value = query.trim();
	if (value === '') return;

	await goto(`${path}?q=${encodeURIComponent(value)}`);
}
