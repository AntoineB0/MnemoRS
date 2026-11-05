/** @type {import("@sveltejs/vite-plugin-svelte").SvelteConfig} */
export default {
  vitePlugin: {
    // Le module CSS virtuel de Vite peut être demandé avant que le composant
    // soit compilé : Vite sert alors le fichier .svelte brut comme feuille de
    // style et le layout perd tout son CSS. L'injection via le JS supprime
    // cette course (le CSS n'est plus un module séparé).
    emitCss: false,
  },
}
