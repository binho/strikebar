// Writes the image-pixel box of every [data-r] element into #boxes, one per line:
//   name cx cy w h angle
// Boxes are padded by data-pad (CSS px; default 2 across and 4 up and down, so
// they don't reach into neighbouring letters) and follow the rotation of
// .page (data-angle, degrees clockwise). The render script reads them back with
// Chrome's --dump-dom and substitutes them into the scene scripts.
(function () {
  const dpr = parseFloat(document.body.dataset.dpr || "2");
  const page = document.querySelector(".page");
  const angle = parseFloat(page.dataset.angle || "0");
  const transform = page.style.transform;
  page.style.transform = "none"; // measure the unrotated layout
  const pr = page.getBoundingClientRect();
  const pcx = pr.left + pr.width / 2;
  const pcy = pr.top + pr.height / 2;
  const t = (angle * Math.PI) / 180;
  const round = (v) => Math.round(v * 10) / 10;
  const lines = [];
  document.querySelectorAll("[data-r]").forEach((el) => {
    const r = el.getBoundingClientRect();
    const padX = parseFloat(el.dataset.pad || "2");
    const padY = parseFloat(el.dataset.pad || "4");
    const dx = r.left + r.width / 2 - pcx;
    const dy = r.top + r.height / 2 - pcy;
    const cx = pcx + dx * Math.cos(t) - dy * Math.sin(t);
    const cy = pcy + dx * Math.sin(t) + dy * Math.cos(t);
    lines.push(
      [
        el.dataset.r,
        round(cx * dpr),
        round(cy * dpr),
        round((r.width + 2 * padX) * dpr),
        round((r.height + 2 * padY) * dpr),
        angle,
      ].join(" ")
    );
  });
  page.style.transform = transform || `rotate(${angle}deg)`;
  document.getElementById("boxes").textContent = lines.join("\n");
})();
