const dashboard = document.querySelector("#sound-dashboard");
const find = (selector) => dashboard.querySelector(selector);
const bands = [...dashboard.querySelectorAll("#eq-bands input[type=range]")];
const points = [...dashboard.querySelectorAll("[data-point]")];
const chart = find("#eq-chart");
const canvas = find("#eq-curve");
const status = find("#controller-status");
const error = find("#audio-error");
const preset = find("#eq");
const apply = find("#apply-eq");
const reset = find("#reset-bands");
const refresh = find("#refresh-audio");
const mute = find("#mute-control");
const committed = { sound: null, tuning: null };
let busy = false;
let dirty = false;

dashboard.dataset.enhanced = "";
for (const element of [chart, reset, refresh]) element.hidden = false;

function decode(report) {
    return Object.fromEntries(report.trim().split(/\s+/).map((field) => field.split("=")));
}

function actualLevels() {
    return committed.tuning?.bands?.split(",").map(Number) ?? [0, 0, 0, 0, 0];
}

function renderCurve() {
    const context = canvas.getContext("2d");
    if (!context) return;
    const { width, height } = canvas.getBoundingClientRect();
    if (!width || !height) return;
    const scale = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(width * scale);
    canvas.height = Math.round(height * scale);
    context.scale(scale, scale);
    const inset = 15;
    const span = height - 2 * inset;
    const y = (level) => inset + (10 - level) * span / 20;
    const coordinates = bands.map((slider, index) => [inset + index * (width - 2 * inset) / 4, y(Number(slider.value))]);

    for (const level of [10, 5, 0, -5, -10]) {
        context.strokeStyle = level === 0 ? "#55536c" : "#242637";
        context.beginPath();
        context.moveTo(0, y(level));
        context.lineTo(width, y(level));
        context.stroke();
    }
    context.strokeStyle = "#242637";
    coordinates.forEach(([x, level], index) => {
        context.beginPath();
        context.moveTo(x, inset);
        context.lineTo(x, height - inset);
        context.stroke();
        points[index].style.left = `${canvas.offsetLeft + x}px`;
        points[index].style.top = `${level}px`;
    });

    function path() {
        context.beginPath();
        context.moveTo(0, coordinates[0][1]);
        context.lineTo(...coordinates[0]);
        for (let index = 1; index < coordinates.length; index++) {
            const [beforeX, beforeY] = coordinates[index - 1];
            const [x, nextY] = coordinates[index];
            const middle = (beforeX + x) / 2;
            context.bezierCurveTo(middle, beforeY, middle, nextY, x, nextY);
        }
        context.lineTo(width, coordinates.at(-1)[1]);
    }

    path();
    context.lineTo(width, height - inset);
    context.lineTo(0, height - inset);
    context.closePath();
    const tint = context.createLinearGradient(0, 0, 0, height);
    tint.addColorStop(0, "#aa95ff60");
    tint.addColorStop(1, "#aa95ff0c");
    context.fillStyle = tint;
    context.fill();
    path();
    context.strokeStyle = "#bdaaff";
    context.lineWidth = 2.5;
    context.stroke();
}

function showBands() {
    bands.forEach((slider, index) => {
        const value = Number(slider.value);
        const label = value > 0 ? `+${value}` : String(value);
        find(`[data-band-value="${index}"]`).textContent = label;
        slider.setAttribute("aria-valuetext", `${label} steps`);
        slider.style.setProperty("--gain", `${(value + 10) * 5}%`);
    });
    find("#curve-caption").textContent = dirty || !committed.tuning?.bands ? "Custom curve preview · steps" : "Custom curve · steps";
    renderCurve();
}

function setAvailability() {
    const soundReady = Boolean(committed.sound);
    const tuningReady = Boolean(committed.tuning);
    dashboard.querySelectorAll('[data-live="sound"] button').forEach((button) => button.disabled = busy || !soundReady);
    dashboard.querySelectorAll("[data-commit]").forEach((select) => select.disabled = busy || !tuningReady);
    for (const band of bands) band.disabled = busy || !tuningReady;
    apply.disabled = busy || !tuningReady || !dirty;
    reset.disabled = busy || !tuningReady;
    refresh.disabled = busy;
    chart.dataset.disabled = String(busy || !tuningReady);
    const state = busy ? "pending" : soundReady && tuningReady && error.hidden ? "ready" : "offline";
    status.dataset.state = state;
    status.textContent = { pending: "Syncing", ready: "Controller online", offline: "Readback unavailable" }[state];
    find("#eq-state").textContent = dirty ? "Unapplied changes" : tuningReady ? "Matches speaker settings" : "Waiting for controller";
}

function update(scope, values, preserveDraft = false) {
    committed[scope] = values;
    if (scope === "sound") {
        const [level, maximum] = values.volume.split("/").map(Number);
        find("#volume-level").textContent = level;
        find("#volume-maximum").textContent = maximum;
        const meter = find("#volume-meter");
        meter.max = maximum;
        meter.value = level;
        const muted = values.mute === "on";
        mute.value = muted ? "unmute" : "mute";
        mute.textContent = muted ? "Unmute" : "Mute";
        mute.setAttribute("aria-pressed", String(muted));
        return;
    }
    preset.value = `eq-${values.eq}`;
    find("#clear").value = values.clear === "1" ? "clear-on" : "clear-off";
    find("#wide").value = values.wide === "1" ? "wide-high" : "wide-normal";
    if (!preserveDraft) {
        actualLevels().forEach((level, index) => bands[index].value = level);
        dirty = false;
    }
    showBands();
}

async function query(scope, form) {
    const response = await fetch(`/${scope}?live=1`, form ? { method: "POST", body: form } : {});
    if (response.status === 401) {
        window.location.assign("/login");
        throw new Error("Sign in again to continue.");
    }
    const report = await response.text();
    if (!response.ok) throw new Error(report);
    const values = decode(report);
    if (scope === "sound" && !/^\d+\/\d+$/.test(values.volume)) throw new Error("Volume readback is incomplete.");
    if (scope === "tuning" && !/^[0-5]$/.test(values.eq)) throw new Error("Tuning readback is incomplete.");
    return values;
}

async function run(task) {
    if (busy) return;
    busy = true;
    error.hidden = true;
    setAvailability();
    try {
        await task();
    } catch (failure) {
        error.textContent = failure.message;
        error.hidden = false;
    } finally {
        busy = false;
        setAvailability();
    }
}

function sync(onlyMissing = false) {
    return run(async () => {
        const failures = [];
        for (const scope of ["sound", "tuning"]) {
            if (onlyMissing && committed[scope]) continue;
            try {
                update(scope, await query(scope), dirty);
            } catch (failure) {
                failures.push(failure.message);
            }
        }
        if (failures.length) throw new Error([...new Set(failures)].join(" "));
    });
}

function markDraft() {
    dirty = bands.some((band, index) => Number(band.value) !== actualLevels()[index]) || committed.tuning?.eq !== "5";
    showBands();
    setAvailability();
}

bands.forEach((band) => band.addEventListener("input", markDraft));
for (const point of points) {
    const band = bands[Number(point.dataset.point)];
    const adjust = (event) => {
        if (!point.hasPointerCapture(event.pointerId)) return;
        const rect = canvas.getBoundingClientRect();
        const position = (event.clientY - rect.top - 15) / (rect.height - 30);
        band.value = Math.round(10 - position * 20);
        markDraft();
    };
    point.addEventListener("pointerdown", (event) => {
        if (band.disabled || event.button !== 0) return;
        event.preventDefault();
        point.setPointerCapture(event.pointerId);
        adjust(event);
    });
    point.addEventListener("pointermove", adjust);
}

reset.addEventListener("click", () => {
    bands.forEach((band) => band.value = 0);
    markDraft();
});
refresh.addEventListener("click", () => sync());
dashboard.querySelectorAll("[data-commit]").forEach((select) => {
    select.addEventListener("change", () => select.form.requestSubmit());
});

dashboard.querySelectorAll("[data-live]").forEach((form) => {
    form.addEventListener("submit", (event) => {
        event.preventDefault();
        if (busy) return;
        const scope = form.dataset.live;
        const fields = new URLSearchParams(new FormData(form, event.submitter || undefined));
        const keepDraft = dirty && /^(clear|wide)-/.test(fields.get("action") || "");
        run(async () => {
            try {
                update(scope, await query(scope, fields), keepDraft);
            } catch (failure) {
                if (committed[scope]) update(scope, committed[scope], dirty);
                throw failure;
            }
        });
    });
});

new ResizeObserver(renderCurve).observe(canvas);
for (const scope of ["sound", "tuning"]) {
    if (dashboard.dataset[scope]) update(scope, decode(dashboard.dataset[scope]));
}
showBands();
sync(true);
