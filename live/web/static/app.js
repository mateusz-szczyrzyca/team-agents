// team-agents web interface: the slide-out menu, maximized views, full screen, and tidy live lists.
// htmx (vendor/) does the rest. No framework, no build.
(function () {
  "use strict";
  var MENU_KEY = "team-agents-menu"; // "1": the menu stays open

  function menuOpen(open) {
    document.body.classList.toggle("menu-open", open);
    var btn = document.getElementById("menu-btn");
    if (btn) btn.setAttribute("aria-expanded", open ? "true" : "false");
    var scrim = document.getElementById("scrim");
    if (scrim) scrim.hidden = !open;
    try { localStorage.setItem(MENU_KEY, open ? "1" : "0"); } catch (e) { /* storage disabled: the menu simply is not remembered */ }
  }

  function isMax() { return document.body.classList.contains("max"); }

  // maximized: the view covers the header and the menu; kept in the URL (?max=1) so a link can open it that way
  function setMax(on) {
    document.body.classList.toggle("max", on);
    if (on) menuOpen(false);
    syncMaxURL();
  }

  function syncMaxURL() {
    var u = new URL(location.href);
    if (isMax()) u.searchParams.set("max", "1"); else u.searchParams.delete("max");
    if (u.href !== location.href) history.replaceState(history.state, "", u);
  }

  function markCurrent() {
    var view = document.getElementById("view");
    var name = view ? view.getAttribute("data-view") : "";
    document.querySelectorAll("#menu a").forEach(function (a) {
      var views = (a.getAttribute("data-views") || "").split(" ");
      if (views.indexOf(name) >= 0) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
    });
  }

  function restore() {
    var open = false;
    try { open = localStorage.getItem(MENU_KEY) === "1"; } catch (e) { /* see menuOpen */ }
    menuOpen(open && !isMax());
    markCurrent();
    var acts = document.querySelector("ol.acts");
    if (acts) window.scrollTo(0, document.body.scrollHeight);
  }

  document.addEventListener("click", function (e) {
    var t = e.target.closest("#menu-btn, #scrim, #menu a, [data-act]");
    if (!t) return;
    if (t.id === "menu-btn") menuOpen(!document.body.classList.contains("menu-open"));
    else if (t.id === "scrim") menuOpen(false);
    else if (t.matches("#menu a")) { if (window.innerWidth < 1200) menuOpen(false); }
    else if (t.getAttribute("data-act") === "max") setMax(!isMax());
    else if (t.getAttribute("data-act") === "fullscreen") {
      if (document.fullscreenElement) document.exitFullscreen();
      else if (document.documentElement.requestFullscreen) {
        document.documentElement.requestFullscreen().catch(function (err) { console.warn("full screen refused:", err); });
      }
    }
  });

  document.addEventListener("keydown", function (e) {
    if (e.ctrlKey || e.metaKey || e.altKey || e.target.closest("input, textarea, select")) return;
    if (e.key === "m") menuOpen(!document.body.classList.contains("menu-open"));
    else if (e.key === "f") setMax(!isMax());
    else if (e.key === "Escape") { if (isMax()) setMax(false); else menuOpen(false); }
  });

  // live lists: the dashboard keeps the newest 200 events; the activity follows new entries when you are at the end
  var follow = true;
  document.addEventListener("htmx:sseBeforeMessage", function () {
    follow = window.innerHeight + window.scrollY >= document.body.scrollHeight - 120;
  });
  document.addEventListener("htmx:sseMessage", function () {
    document.querySelectorAll("ol.events").forEach(function (ol) {
      var placeholder = ol.querySelector("li.dim");
      if (placeholder && ol.children.length > 1) placeholder.remove();
      while (ol.children.length > 200) ol.lastElementChild.remove();
    });
    if (follow && document.querySelector("ol.acts, ol.msgs")) window.scrollTo(0, document.body.scrollHeight);
  });

  document.addEventListener("htmx:afterSettle", restore);
  document.addEventListener("htmx:pushedIntoHistory", syncMaxURL);
  document.addEventListener("DOMContentLoaded", restore);
})();
