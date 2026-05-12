import { defineConfig } from "vitepress";

export default defineConfig({
  title: "Runx",
  description: "A fast, native macOS launcher with Lua plugins",
  base: "/runx/",
  cleanUrls: true,

  head: [["link", { rel: "icon", href: "/runx/favicon.ico" }]],

  themeConfig: {
    logo: "/logo.png",

    nav: [
      { text: "Guide", link: "/guide/getting-started" },
      { text: "Configuration", link: "/guide/configuration" },
      { text: "Plugins", link: "/guide/plugins" },
      { text: "☕ Sponsor", link: "https://ko-fi.com/magnickolas" },
    ],

    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Getting Started", link: "/guide/getting-started" },
          { text: "Configuration", link: "/guide/configuration" },
          { text: "Plugins", link: "/guide/plugins" },
        ],
      },
    ],

    socialLinks: [{ icon: "github", link: "https://github.com/sloppish/runx" }],

    search: {
      provider: "local",
    },

    footer: {
      message: 'Built by <a href="https://github.com/sloppish">Sloppish</a>',
    },

  },
});
