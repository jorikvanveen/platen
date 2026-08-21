import { env } from "$env/dynamic/public"

console.log(env.PUBLIC_PLATEN_BACKEND_URL)
export const API_URL = env.PUBLIC_PLATEN_BACKEND_URL ?? "/api"
export const PAGE_SIZE = 100;
