import { defineConfig } from "vitepress";

const SITE_HOST = "https://sloppish.github.io";
const SITE_URL = `${SITE_HOST}/runx`;

function canonicalUrl(page: string): string {
  if (page === "404.md") {
    return `${SITE_URL}/404`;
  }

  const normalizedPage = page === "index.md" ? "" : page.replace(/\.md$/, "");
  return normalizedPage ? `${SITE_URL}/${normalizedPage}` : `${SITE_URL}/`;
}

export default defineConfig({
  title: "Runx",
  description: "A fast, native macOS launcher",
  base: "/runx/",
  cleanUrls: true,
  sitemap: {
    hostname: SITE_HOST,
    transformItems(items) {
      return items.map((item) => {
        const url = item.url === "/" ? "/runx/" : `/runx/${item.url.replace(/^\/+/, "")}`;
        return { ...item, url };
      });
    },
  },

  head: [
    ["link", { rel: "icon", type: "image/x-icon", sizes: "48x48", href: "/runx/favicon.ico" }],
    ["link", { rel: "apple-touch-icon", sizes: "180x180", href: "/runx/apple-touch-icon.png" }],
    ["meta", { property: "og:site_name", content: "Runx" }],
    ["meta", { property: "og:image", content: `${SITE_URL}/og-icon.png` }],
    ["meta", { property: "og:image:width", content: "280" }],
    ["meta", { property: "og:image:height", content: "280" }],
    ["meta", { name: "twitter:card", content: "summary" }],
  ],

  transformHead({ page, title, description }) {
    const pageTitle = title || "Runx";
    const pageDescription = description || "A fast, native macOS launcher";
    const url = canonicalUrl(page);

    return [
      ["link", { rel: "canonical", href: url }],
      ["meta", { name: "robots", content: "index,follow" }],
      ["meta", { property: "og:type", content: "website" }],
      ["meta", { property: "og:title", content: pageTitle }],
      ["meta", { property: "og:description", content: pageDescription }],
      ["meta", { property: "og:url", content: url }],
      ["meta", { name: "twitter:title", content: pageTitle }],
      ["meta", { name: "twitter:description", content: pageDescription }],
      ["meta", { name: "twitter:image", content: `${SITE_URL}/og-icon.png` }],
    ];
  },

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

    socialLinks: [{ icon: { svg: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/></svg>' }, link: "https://github.com/sloppish/runx" }],

    search: {
      provider: "local",
    },

    footer: {
      message: 'Built by <a href="https://github.com/sloppish">Sloppish</a>',
    },

  },
});
