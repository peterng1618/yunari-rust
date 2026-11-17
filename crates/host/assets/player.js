// The phone player paints the host's own display list on a canvas.
//
// The host decides what is drawn; this file only knows how to draw a rect, an
// ellipse, a path, a run of text and a picture. Nothing here reads a theme.

const canvas = document.getElementById("stage");
const context = canvas.getContext("2d");
const stateLabel = document.getElementById("state");
const pictures = new Map();
const videos = new Map();

let frame = null;
let themeId = null;
let rotation = 0;
let shownAt = 0;

function say(text, seconds = 0) {
  stateLabel.textContent = text;
  stateLabel.classList.add("shown");
  shownAt = seconds ? performance.now() + seconds * 1000 : 0;
  if (!seconds) return;
  setTimeout(() => {
    if (performance.now() >= shownAt) stateLabel.classList.remove("shown");
  }, seconds * 1000);
}

function colour(value, alpha = 1) {
  if (!value) return `rgba(0,0,0,${alpha})`;
  return `rgba(${value.r},${value.g},${value.b},${(value.a / 255) * alpha})`;
}

function paint(ctx, value) {
  if (!value) return "#000";
  if (value.Solid) return colour(value.Solid);
  if (value.Linear) {
    const { from, to, stops } = value.Linear;
    const gradient = ctx.createLinearGradient(from[0], from[1], to[0], to[1]);
    for (const stop of stops) gradient.addColorStop(stop.offset, colour(stop.color));
    return gradient;
  }
  return "#000";
}

function shapePath(ctx, shape) {
  if (shape.Rect) {
    const { width, height, corner_radius: radius } = shape.Rect;
    ctx.beginPath();
    if (radius > 0 && ctx.roundRect) ctx.roundRect(0, 0, width, height, radius);
    else ctx.rect(0, 0, width, height);
    return;
  }
  if (shape.Ellipse) {
    const { width, height } = shape.Ellipse;
    ctx.beginPath();
    ctx.ellipse(width / 2, height / 2, width / 2, height / 2, 0, 0, Math.PI * 2);
    return;
  }
  if (shape.Path) {
    ctx.beginPath();
    for (const command of shape.Path.commands) {
      if (command.MoveTo) ctx.moveTo(command.MoveTo[0], command.MoveTo[1]);
      else if (command.LineTo) ctx.lineTo(command.LineTo[0], command.LineTo[1]);
      else if (command.QuadTo) ctx.quadraticCurveTo(command.QuadTo[0], command.QuadTo[1], command.QuadTo[2], command.QuadTo[3]);
      else if (command.CubicTo) ctx.bezierCurveTo(command.CubicTo[0], command.CubicTo[1], command.CubicTo[2], command.CubicTo[3], command.CubicTo[4], command.CubicTo[5]);
      else if (command.Close !== undefined) ctx.closePath();
    }
  }
}

function picture(source) {
  const url = `/api/themes/${encodeURIComponent(themeId)}/assets/${source.path.split("/").map(encodeURIComponent).join("/")}`;
  let image = pictures.get(url);
  if (!image) {
    image = new Image();
    image.src = url;
    pictures.set(url, image);
  }
  return image;
}

/** A theme clip is its own element: the canvas draws whatever frame it holds. */
function video(source) {
  const url = `/api/themes/${encodeURIComponent(themeId)}/assets/${source.path.split("/").map(encodeURIComponent).join("/")}`;
  let clip = videos.get(url);
  if (!clip) {
    clip = document.createElement("video");
    clip.muted = true;
    clip.loop = source.video_loop !== false;
    clip.playsInline = true;
    clip.autoplay = true;
    clip.src = url;
    clip.play().catch(() => undefined);
    clip.addEventListener("loadeddata", () => render(), { once: true });
    clip.addEventListener("timeupdate", () => render());
    videos.set(url, clip);
  }
  return clip;
}

function media(source) {
  return source.video ? video(source) : picture(source);
}

function mediaSize(source, element) {
  return source.video
    ? { width: element.videoWidth, height: element.videoHeight }
    : { width: element.naturalWidth, height: element.naturalHeight };
}

function mediaReady(source, element) {
  const size = mediaSize(source, element);
  return size.width > 0 && size.height > 0;
}

function drawText(layout) {
  const clip = layout.clip;
  if (clip) context.rect(0, 0, clip[0], clip[1]);
  for (const segment of layout.segments) {
    const style = segment.style;
    context.save();
    context.translate(segment.origin[0], segment.origin[1]);
    context.font = `${style.italic ? "italic " : ""}${style.weight} ${style.size}px ${style.family || "sans-serif"}`;
    context.fillStyle = colour(style.color);
    context.textBaseline = "top";
    context.fillText(segment.text, 0, 0);
    context.restore();
  }
}

function drawImage(source) {
  const element = media(source);
  const { width, height } = source;
  const fit = source.fit;
  const draw = (w, h, dx = 0, dy = 0) => {
    if (mediaReady(source, element)) context.drawImage(element, dx, dy, w, h);
  };
  if (fit === "Stretch" || fit === "stretch") return draw(width, height);
  if (!mediaReady(source, element)) {
    const event = source.video ? "loadeddata" : "load";
    element.addEventListener(event, () => render(), { once: true });
    return;
  }
  const size = mediaSize(source, element);
  const cover = fit === "Cover" || fit === "cover";
  const ratio = cover
    ? Math.max(width / size.width, height / size.height)
    : Math.min(width / size.width, height / size.height);
  const w = size.width * ratio;
  const h = size.height * ratio;
  draw(w, h, (width - w) / 2, (height - h) / 2);
}

/** The page's own picture or clip, covering the artboard. */
function drawBackdrop(backdrop) {
  if (!backdrop) return;
  const element = media(backdrop);
  if (!mediaReady(backdrop, element)) {
    const event = backdrop.video ? "loadeddata" : "load";
    element.addEventListener(event, () => render(), { once: true });
    return;
  }
  const size = mediaSize(backdrop, element);
  const ratio = Math.max(frame.width / size.width, frame.height / size.height);
  const w = size.width * ratio;
  const h = size.height * ratio;
  context.drawImage(element, (frame.width - w) / 2, (frame.height - h) / 2, w, h);
}

function drawOp(op) {
  const { transform: t } = op;
  context.save();
  context.transform(t.a, t.b, t.c, t.d, t.e, t.f);
  const shape = op.shape;

  if (shape.Text !== undefined) {
    drawText(shape.Text);
  } else if (shape.Image) {
    drawImage(shape.Image);
  } else if (shape.Glass) {
    // ponytail: a frosted panel paints as a light veil; the blur is the GPUI
    // backend's, and a canvas player can gain a backdrop filter when panels
    // grow a real backdrop pass.
    shapePath(context, shape.Glass.shape);
    context.fillStyle = "rgba(255,255,255,0.08)";
    context.fill();
  } else if (shape.Line) {
    context.beginPath();
    context.moveTo(0, 0);
    context.lineTo(shape.Line.length, 0);
    if (op.stroke) {
      context.strokeStyle = paint(context, op.stroke.paint);
      context.lineWidth = op.stroke.width;
      context.stroke();
    }
  } else if (shape.Path) {
    shapePath(context, shape);
    if (op.fill) {
      context.fillStyle = paint(context, op.fill);
      context.fill();
    }
    if (op.stroke) {
      context.strokeStyle = paint(context, op.stroke.paint);
      context.lineWidth = op.stroke.width;
      context.stroke();
    }
  } else if (shape.Rect || shape.Ellipse) {
    shapePath(context, shape);
    if (op.fill) {
      context.fillStyle = paint(context, op.fill);
      context.fill();
    }
    if (op.stroke) {
      context.strokeStyle = paint(context, op.stroke.paint);
      context.lineWidth = op.stroke.width;
      context.stroke();
    }
  }
  context.restore();
}

function render() {
  if (!frame) return;
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.round(canvas.clientWidth * ratio);
  canvas.height = Math.round(canvas.clientHeight * ratio);
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.fillStyle = "#000";
  context.fillRect(0, 0, canvas.width, canvas.height);

  const fit = Math.min(canvas.width / frame.width, canvas.height / frame.height);
  context.translate((canvas.width - frame.width * fit) / 2, (canvas.height - frame.height * fit) / 2);
  context.scale(fit, fit);
  if (rotation) {
    context.translate(frame.width / 2, frame.height / 2);
    context.rotate((rotation * Math.PI) / 180);
    context.translate(-frame.width / 2, -frame.height / 2);
  }
  if (frame.background) {
    context.fillStyle = paint(context, frame.background);
    context.fillRect(0, 0, frame.width, frame.height);
  }
  drawBackdrop(frame.backdrop);
  for (const op of frame.ops) drawOp(op);
}

async function loadPublished() {
  const response = await fetch("/api/published", { headers: { accept: "application/json" } });
  if (response.status === 401 || response.status === 403) {
    location.replace("/pair?reason=unpaired");
    return null;
  }
  if (!response.ok) throw new Error("published");
  return response.json();
}

async function start() {
  if (new URLSearchParams(location.search).has("session")) {
    const token = new URLSearchParams(location.search).get("session");
    const exchanged = await fetch("/api/session", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    });
    if (exchanged.ok) {
      location.replace("/");
      return;
    }
  }

  const published = await loadPublished();
  if (!published) return;
  themeId = published.display.theme;
  rotation = published.display.rotation || 0;
  if (!themeId) {
    say("This host has no theme for this display yet.", 8);
  }
  connect();
  setInterval(async () => {
    try {
      const next = await loadPublished();
      if (!next) return;
      if (next.display.theme !== themeId) {
        themeId = next.display.theme;
        frame = null;
        connect();
      }
      rotation = next.display.rotation || 0;
      render();
    } catch {
      // A poll that fails leaves the current frame on screen.
    }
  }, 2000);
}

let stream = null;
function connect() {
  if (stream) stream.close();
  stream = new EventSource("/ws?keys=");
  stream.addEventListener("samples", (event) => {
    try {
      const batch = JSON.parse(event.data);
      if (batch.version !== 1) {
        say(`this display speaks protocol 1 and the host sent ${batch.version}. Update whichever is older.`, 30);
        return;
      }
      if (batch.frame) {
        frame = batch.frame;
        render();
        if (stateLabel.classList.contains("shown") && !shownAt) return;
        stateLabel.classList.remove("shown");
      }
    } catch {
      say("This display could not read the host's frame.", 10);
    }
  });
  stream.addEventListener("open", () => say("", 0));
  stream.addEventListener("error", () => {
    if (stream.readyState === EventSource.CLOSED) return;
    say("Connecting…");
  });
}

async function wake() {
  try {
    if (navigator.wakeLock) {
      const lock = await navigator.wakeLock.request("screen");
      document.addEventListener("visibilitychange", async () => {
        if (document.visibilityState === "visible") await navigator.wakeLock.request("screen");
      });
      void lock;
    }
  } catch {
    // A browser that refuses a wake lock still shows the theme.
  }
}

canvas.addEventListener("click", async () => {
  if (!document.fullscreenElement) {
    try {
      await document.documentElement.requestFullscreen();
    } catch {
      // Fullscreen is the browser's choice.
    }
  }
});

new ResizeObserver(render).observe(canvas);
window.addEventListener("resize", render);
void wake();
start().catch(() => say("Connecting…", 0));
