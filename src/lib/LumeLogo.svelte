<script lang="ts">
  let { size = 28 }: { size?: number } = $props();
</script>

<span class="lume-logo" style:--logo-size={`${size}px`} aria-hidden="true">
  <img src="/lume.svg" alt="" />
</span>

<style>
  .lume-logo {
    width: var(--logo-size);
    height: var(--logo-size);
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
  }

  img {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: contain;
  }
</style>
