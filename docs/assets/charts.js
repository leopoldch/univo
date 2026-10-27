// Small SVG charts for the documentation: horizontal bars and x/y lines.
// Each chart is a <div class="viz-root" data-chart="bars|xy"> holding its
// data in a <script type="application/json"> child.

(function () {
  const NS = "http://www.w3.org/2000/svg";

  function el(name, attrs, parent) {
    const node = document.createElementNS(NS, name);
    for (const [key, value] of Object.entries(attrs || {})) node.setAttribute(key, value);
    if (parent) parent.appendChild(node);
    return node;
  }

  function format(value, kind) {
    if (kind === "pct") {
      const percent = value * 100;
      return (Number.isInteger(Math.round(percent * 10) / 10) ? percent.toFixed(0) : percent.toFixed(1)) + "%";
    }
    if (kind === "pct1") return (value * 100).toFixed(1) + "%";
    if (kind === "s") return value.toLocaleString("en-US", { maximumFractionDigits: 2 }) + " s";
    if (kind === "MiB") return value.toLocaleString("en-US") + " MiB";
    if (kind === "num2") return value.toFixed(2);
    return value.toLocaleString("en-US");
  }

  function color(slot) {
    return slot === "ref" ? "var(--viz-ref)" : `var(--viz-${slot || 1})`;
  }

  function header(root, spec) {
    const title = document.createElement("p");
    title.className = "viz-title";
    title.textContent = spec.title;
    root.appendChild(title);
    if (spec.subtitle) {
      const subtitle = document.createElement("p");
      subtitle.className = "viz-subtitle";
      subtitle.textContent = spec.subtitle;
      root.appendChild(subtitle);
    }
  }

  function tooltip(root) {
    const tip = document.createElement("div");
    tip.className = "viz-tip";
    root.appendChild(tip);
    return {
      show(html, x, y) {
        tip.innerHTML = html;
        const width = root.clientWidth;
        tip.style.left = Math.min(Math.max(x + 12, 0), width - tip.offsetWidth - 4) + "px";
        tip.style.top = Math.max(y - tip.offsetHeight - 10, 0) + "px";
        tip.style.opacity = 1;
      },
      hide() {
        tip.style.opacity = 0;
      },
    };
  }

  function table(root, head, rows) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = "Data";
    details.appendChild(summary);
    const tableNode = document.createElement("table");
    const tr = tableNode.insertRow();
    head.forEach((cell) => {
      const th = document.createElement("th");
      th.textContent = cell;
      tr.appendChild(th);
    });
    rows.forEach((row) => {
      const line = tableNode.insertRow();
      row.forEach((cell) => (line.insertCell().textContent = cell));
    });
    details.appendChild(tableNode);
    root.appendChild(details);
  }

  function pointer(svg, event) {
    const box = svg.getBoundingClientRect();
    const scale = svg.viewBox.baseVal.width / box.width;
    // SVG elements have no offsetTop: measure the chart's offset in its root.
    const top = box.top - svg.parentNode.getBoundingClientRect().top;
    return { x: (event.clientX - box.left) * scale, y: (event.clientY - box.top) * scale, scale, top };
  }

  // Drawing at the container's width keeps text at its size on phones.
  function widthOf(root) {
    return Math.round(Math.min(Math.max(root.clientWidth, 300), 680));
  }

  function bars(root, spec) {
    header(root, spec);
    const width = widthOf(root);
    // On narrow screens each label sits above its bar.
    const stacked = width < 480;
    const row = stacked ? 46 : 34;
    const left = stacked ? 0 : spec.labelWidth || 170;
    const shift = stacked ? 14 : 0;
    const right = 70;
    const height = spec.items.length * row + 8;
    const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": spec.title }, root);
    const max = Math.max(...spec.items.map((item) => item.value));
    const tip = tooltip(root);
    spec.items.forEach((item, index) => {
      const y = index * row + 6;
      const length = Math.max(((width - left - right) * item.value) / max, 2);
      el("text", stacked ? { x: 0, y: y + 8, class: "label" } : { x: left - 10, y: y + 15, "text-anchor": "end", class: "label" }, svg).textContent = item.label;
      const bar = el("rect", { x: left, y: y + 4 + shift, width: length, height: 16, rx: 4, fill: color(item.color) }, svg);
      el("text", { x: left + length + 8, y: y + 16 + shift, class: "value" }, svg).textContent = format(item.value, spec.format);
      const hit = el("rect", { x: 0, y, width, height: row, fill: "transparent" }, svg);
      hit.addEventListener("mousemove", (event) => {
        const at = pointer(svg, event);
        bar.setAttribute("opacity", 0.8);
        tip.show(`<b>${item.label}</b><br>${format(item.value, spec.format)}${item.note ? "<br>" + item.note : ""}`,
          at.x / at.scale, at.top + (y + 4) / at.scale);
      });
      hit.addEventListener("mouseleave", () => {
        bar.removeAttribute("opacity");
        tip.hide();
      });
    });
    table(root, ["", spec.valueName || "Value"], spec.items.map((item) => [item.label, format(item.value, spec.format)]));
  }

  function xy(root, spec) {
    header(root, spec);
    if (spec.series.length > 1) {
      const legend = document.createElement("div");
      legend.className = "viz-legend";
      spec.series.forEach((series) => {
        const entry = document.createElement("span");
        entry.style.setProperty("--swatch", color(series.color));
        entry.textContent = series.name;
        legend.appendChild(entry);
      });
      root.appendChild(legend);
    }
    const width = widthOf(root);
    const height = width < 500 ? 240 : 300;
    const pad = { left: 46, right: Math.min(spec.rightPad || 110, width * 0.22), top: 12, bottom: 38 };
    const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": spec.title }, root);
    const sx = (v) => pad.left + ((v - spec.x.min) / (spec.x.max - spec.x.min)) * (width - pad.left - pad.right);
    const sy = (v) => height - pad.bottom - ((v - spec.y.min) / (spec.y.max - spec.y.min)) * (height - pad.top - pad.bottom);

    spec.y.ticks.forEach((t) => {
      el("line", { x1: pad.left, x2: width - pad.right, y1: sy(t), y2: sy(t), stroke: "var(--viz-grid)", "stroke-width": 1 }, svg);
      el("text", { x: pad.left - 8, y: sy(t) + 3, "text-anchor": "end", class: "tick" }, svg).textContent = format(t, spec.y.format);
    });
    el("line", { x1: pad.left, x2: width - pad.right, y1: sy(spec.y.min), y2: sy(spec.y.min), stroke: "var(--viz-axis)" }, svg);
    spec.x.ticks.forEach((t) => {
      el("text", { x: sx(t), y: height - pad.bottom + 16, "text-anchor": "middle", class: "tick" }, svg).textContent = format(t, spec.x.format);
    });
    el("text", { x: (pad.left + width - pad.right) / 2, y: height - 4, "text-anchor": "middle", class: "label" }, svg).textContent = spec.x.label;
    el("text", { x: 12, y: (pad.top + height - pad.bottom) / 2, "text-anchor": "middle", class: "label", transform: `rotate(-90 12 ${(pad.top + height - pad.bottom) / 2})` }, svg).textContent = spec.y.label;

    if (spec.marker) {
      const x = sx(spec.marker.x);
      el("line", { x1: x, x2: x, y1: pad.top, y2: sy(spec.y.min), stroke: "var(--viz-ref)", "stroke-dasharray": "4 4" }, svg);
      el("text", { x: x + 6, y: sy(spec.y.min) - 8, class: "label" }, svg).textContent = spec.marker.label;
    }

    const marks = [];
    spec.series.forEach((series) => {
      const d = series.points.map((p, i) => `${i ? "L" : "M"}${sx(p.x).toFixed(1)},${sy(p.y).toFixed(1)}`).join("");
      el("path", { d, fill: "none", stroke: color(series.color), "stroke-width": 2, "stroke-linejoin": "round" }, svg);
      series.points.forEach((p) => {
        const dot = el("circle", { cx: sx(p.x), cy: sy(p.y), r: 4, fill: color(series.color), stroke: "var(--viz-surface)", "stroke-width": 2 }, svg);
        marks.push({ series, p, dot });
      });
      const last = series.points[series.points.length - 1];
      const label = series.labelAt ? series.points.find((p) => p.x === series.labelAt) : last;
      el("text", { x: sx(label.x) + 8, y: sy(label.y) + (series.labelDy || 4), class: "label" }, svg).textContent = series.label || series.name;
    });

    const tip = tooltip(root);
    const cross = el("line", { y1: pad.top, y2: sy(spec.y.min), stroke: "var(--viz-axis)", opacity: 0 }, svg);
    const hit = el("rect", { x: pad.left, y: pad.top, width: width - pad.left - pad.right, height: height - pad.top - pad.bottom, fill: "transparent" }, svg);
    hit.addEventListener("mousemove", (event) => {
      const at = pointer(svg, event);
      marks.forEach((m) => m.dot.setAttribute("r", 4));
      let chosen;
      if (spec.hover === "index") {
        const xs = spec.series[0].points.map((p) => p.x);
        const nearest = xs.reduce((a, b) => (Math.abs(sx(b) - at.x) < Math.abs(sx(a) - at.x) ? b : a));
        chosen = marks.filter((m) => m.p.x === nearest);
        cross.setAttribute("x1", sx(nearest));
        cross.setAttribute("x2", sx(nearest));
        cross.setAttribute("opacity", 1);
        const lines = chosen.map((m) => `<i style="--swatch:${color(m.series.color)}"></i>${m.series.name}: <b>${format(m.p.y, spec.y.format)}</b>`);
        tip.show(`<b>${spec.x.label} ${format(nearest, spec.x.format)}</b><br>${lines.join("<br>")}`, sx(nearest) / at.scale, at.top + at.y / at.scale);
      } else {
        const best = marks.reduce((a, b) =>
          Math.hypot(sx(b.p.x) - at.x, sy(b.p.y) - at.y) < Math.hypot(sx(a.p.x) - at.x, sy(a.p.y) - at.y) ? b : a);
        chosen = [best];
        tip.show(`<i style="--swatch:${color(best.series.color)}"></i><b>${best.series.name}</b><br>${best.p.label ? best.p.label + "<br>" : ""}${spec.x.label}: ${format(best.p.x, spec.x.format)}<br>${spec.y.label}: ${format(best.p.y, spec.y.format)}`,
          sx(best.p.x) / at.scale, at.top + sy(best.p.y) / at.scale);
      }
      chosen.forEach((m) => m.dot.setAttribute("r", 6));
    });
    hit.addEventListener("mouseleave", () => {
      tip.hide();
      cross.setAttribute("opacity", 0);
      marks.forEach((m) => m.dot.setAttribute("r", 4));
    });

    const head = ["Series", ...(spec.series[0].points[0].label !== undefined ? ["Point"] : []), spec.x.label, spec.y.label];
    const rows = [];
    spec.series.forEach((series) => series.points.forEach((p) =>
      rows.push([series.name, ...(p.label !== undefined ? [p.label] : []), format(p.x, spec.x.format), format(p.y, spec.y.format)])));
    table(root, head, rows);
  }

  function render() {
    document.querySelectorAll(".viz-root[data-chart]:not([data-ready])").forEach((root) => {
      const spec = JSON.parse(root.querySelector('script[type="application/json"]').textContent);
      root.dataset.ready = "1";
      (root.dataset.chart === "bars" ? bars : xy)(root, spec);
    });
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", render);
  else render();
})();
