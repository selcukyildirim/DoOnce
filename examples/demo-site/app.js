// Shared behavior of the DoOnce demo site. The "site version" switch in the
// footer simulates a redesign so self-healing can be tried safely:
//   classic    original markup
//   redesign   ids and classes change, visible text stays the same
//   renamed    buttons are also renamed (needs AI repair)

(function () {
  const params = new URLSearchParams(location.search);
  if (params.has("variant")) {
    localStorage.setItem("demo-variant", params.get("variant"));
    params.delete("variant");
    const qs = params.toString();
    history.replaceState(null, "", location.pathname + (qs ? "?" + qs : ""));
  }
  window.demoVariant = localStorage.getItem("demo-variant") || "classic";

  document.addEventListener("DOMContentLoaded", () => {
    document.querySelectorAll("header nav a").forEach((a) => {
      if (a.getAttribute("href") === location.pathname.split("/").pop()) a.classList.add("current");
    });
    const footer = document.createElement("footer");
    footer.innerHTML = `Demo site · version: <b>${window.demoVariant}</b>
      <a href="?variant=classic">classic</a><a href="?variant=redesign">redesign</a><a href="?variant=renamed">renamed</a>`;
    document.body.appendChild(footer);
  });
})();

function store(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; }
}

function save(key, value) {
  localStorage.setItem(key, JSON.stringify(value));
}
