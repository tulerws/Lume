<script lang="ts">
  let { size = 28 }: { size?: number } = $props();
</script>

<span class="lume-logo" style:--logo-size={`${size}px`} aria-hidden="true">
  <img
    class="light"
    src="/branding/light/lume-32.png"
    srcset="/branding/light/lume-32.png 1x, /branding/light/lume-64.png 2x"
    alt=""
  />
  <img
    class="dark"
    src="/branding/dark/lume-32.png"
    srcset="/branding/dark/lume-32.png 1x, /branding/dark/lume-64.png 2x"
    alt=""
  />
</span>

<style>
  .lume-logo {
    position: relative;
    width: var(--logo-size);
    height: var(--logo-size);
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
  }

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    object-fit: contain;
  }

  .dark { display: none; }

  @media (prefers-color-scheme: dark) {
    .light { display: none; }
    .dark { display: block; }
  }
</style>
