<script lang="ts" module>
  // What the character is showing (mapped from the pill's mode).
  export type Mood = "idle" | "listening" | "thinking" | "success" | "error" | "suggestion" | "alarm" | "sleep";
</script>

<script lang="ts">
  import { onMount } from "svelte";

  // Mimo's little face (setting `character_enabled`): a blob in the theme
  // color whose eyes and mouth react to what Mimo does, and whose eyes
  // follow the pointer at rest. Pure SVG + CSS animations, which only run
  // while the window is shown; Windows' "reduce animations" turns them off.
  let { mood = "idle", size = 22 }: { mood?: Mood; size?: number } = $props();

  let svg = $state<SVGSVGElement>();
  let gaze = $state({ x: 0, y: 0 });

  onMount(() => {
    const follow = (event: PointerEvent) => {
      if (!svg || mood !== "idle") return;
      const box = svg.getBoundingClientRect();
      const dx = event.clientX - (box.left + box.width / 2);
      const dy = event.clientY - (box.top + box.height / 2);
      const length = Math.hypot(dx, dy) || 1;
      const reach = Math.min(1, length / 140) * 1.4;
      gaze = { x: (dx / length) * reach, y: (dy / length) * reach * 0.8 };
    };
    const rest = () => (gaze = { x: 0, y: 0 });
    window.addEventListener("pointermove", follow);
    document.documentElement.addEventListener("pointerleave", rest);
    return () => {
      window.removeEventListener("pointermove", follow);
      document.documentElement.removeEventListener("pointerleave", rest);
    };
  });

  const look = $derived(mood === "idle" ? `translate(${gaze.x.toFixed(2)}px, ${gaze.y.toFixed(2)}px)` : "");

  // Eye geometry (viewBox 24×24).
  const W = 2.4;
  const H = 4.4;
  const EYES = [
    { x: 9.4, side: "l" },
    { x: 14.6, side: "r" },
  ] as const;
  const EYE_Y = 10.6;
</script>

<!-- Rebuilt on each new mood, which replays the one-shot animations (hop, shake). -->
{#key mood}
  <svg
    bind:this={svg}
    class="face"
    data-mood={mood}
    viewBox="0 0 24 24"
    width={size}
    height={size}
    aria-hidden="true"
  >
    <g class="head">
      <rect class="skin" x="1.6" y="3" width="20.8" height="19" rx="9.2" />
      <ellipse class="shine" cx="8" cy="6.6" rx="3.6" ry="1.5" />
      <g class="look" style:transform={look}>
        {#each EYES as eye (eye.side)}
          <g transform={`translate(${eye.x} ${EYE_Y})`}>
            <rect class="shape open ink" x={-W / 2} y={-H / 2} width={W} height={H} rx={W / 2} />
            <path class="shape happy line" d={`M${-W * 0.62} ${H * 0.1} Q0 ${-H * 0.34} ${W * 0.62} ${H * 0.1}`} />
            <path class="shape closed line" d={`M${-W * 0.6} ${H * 0.04} Q0 ${H * 0.2} ${W * 0.6} ${H * 0.04}`} />
            <path
              class="shape sad line"
              d={eye.side === "l"
                ? `M${-W * 0.6} ${H * 0.14} L${W * 0.6} ${-H * 0.1}`
                : `M${-W * 0.6} ${-H * 0.1} L${W * 0.6} ${H * 0.14}`}
            />
            <circle class="shape round ink" cx="0" cy="0" r={W * 0.68} />
          </g>
        {/each}
        <g transform="translate(12 16.2)">
          <path class="mouth m-neutral line" d="M-1.5 -.1 Q0 .8 1.5 -.1" />
          <path class="mouth m-smile line" d="M-2.2 -.6 Q0 1.9 2.2 -.6" />
          <path class="mouth m-frown line" d="M-1.8 .8 Q0 -.9 1.8 .8" />
          <path class="mouth m-think line" d="M-.7 .2 L1.5 -.4" />
          <circle class="mouth m-o line" cx="0" cy=".1" r=".95" />
          <circle class="mouth m-big-o line" cx="0" cy=".2" r="1.4" />
          <circle class="mouth m-tiny-o line" cx="0" cy=".1" r=".55" />
        </g>
      </g>
    </g>
  </svg>
{/key}

<style>
  .face {
    flex-shrink: 0;
    overflow: visible;
    filter: drop-shadow(0 0 4px rgba(var(--accent-rgb), 0.45));
  }

  .face * {
    transform-box: fill-box;
    transform-origin: center;
  }

  .skin {
    fill: var(--accent);
    transition: fill 0.3s ease;
  }

  .shine {
    fill: rgba(255, 255, 255, 0.28);
  }

  .head {
    transform-origin: 50% 60%;
    transition: transform 0.35s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .look {
    transform-box: view-box;
    transition: transform 0.3s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .ink {
    fill: #15151b;
  }

  .line {
    fill: none;
    stroke: #15151b;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .shape,
  .mouth {
    opacity: 0;
    transition: opacity 0.16s ease;
  }

  /* Eyes per mood. */
  .open {
    opacity: 1;
  }
  [data-mood="success"] .open,
  [data-mood="error"] .open,
  [data-mood="alarm"] .open,
  [data-mood="sleep"] .open {
    opacity: 0;
  }
  [data-mood="success"] .happy,
  [data-mood="error"] .sad,
  [data-mood="alarm"] .round,
  [data-mood="sleep"] .closed {
    opacity: 1;
  }

  /* Mouth per mood. */
  [data-mood="idle"] .m-neutral,
  [data-mood="listening"] .m-o,
  [data-mood="thinking"] .m-think,
  [data-mood="success"] .m-smile,
  [data-mood="suggestion"] .m-smile,
  [data-mood="error"] .m-frown,
  [data-mood="alarm"] .m-big-o,
  [data-mood="sleep"] .m-tiny-o {
    opacity: 1;
  }

  /* Blinks now and then. */
  @keyframes blink {
    0%,
    93%,
    100% {
      transform: scaleY(1);
    }
    95.5% {
      transform: scaleY(0.1);
    }
  }
  [data-mood="idle"] .open,
  [data-mood="suggestion"] .open,
  [data-mood="thinking"] .open {
    animation: blink 4.6s infinite;
  }

  /* Listening: wide eyes, a pop, then breathing. */
  @keyframes pop {
    0% {
      transform: scale(1);
    }
    40% {
      transform: scale(1.2) translateY(-1px);
    }
    100% {
      transform: scale(1);
    }
  }
  @keyframes breathe {
    0%,
    100% {
      transform: scale(1);
    }
    50% {
      transform: scale(1.06);
    }
  }
  [data-mood="listening"] .head {
    animation:
      pop 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      breathe 1.3s ease-in-out 0.4s infinite;
  }
  [data-mood="listening"] .open {
    transform: scaleY(1.2);
  }

  /* Thinking: looks up-left, up-right. */
  @keyframes ponder {
    0%,
    100% {
      transform: translate(0, 0);
    }
    18%,
    40% {
      transform: translate(-1.4px, -1.2px);
    }
    58%,
    80% {
      transform: translate(1.4px, -1.2px);
    }
  }
  [data-mood="thinking"] .look {
    animation: ponder 2.6s ease-in-out infinite;
  }

  /* Success: a little hop. */
  @keyframes hop {
    0% {
      transform: translateY(0);
    }
    35% {
      transform: translateY(-3px) scale(1.04, 0.96);
    }
    60% {
      transform: translateY(0) scale(0.96, 1.04);
    }
    100% {
      transform: translateY(0);
    }
  }
  [data-mood="success"] .head {
    animation: hop 0.55s cubic-bezier(0.16, 1, 0.3, 1);
  }

  /* Didn't work: shakes its head. */
  @keyframes shake {
    0%,
    100% {
      transform: translateX(0);
    }
    20% {
      transform: translateX(-2px) rotate(-6deg);
    }
    45% {
      transform: translateX(2px) rotate(6deg);
    }
    70% {
      transform: translateX(-1.2px) rotate(-3deg);
    }
  }
  [data-mood="error"] .head {
    animation: shake 0.5s ease-in-out;
  }

  /* Suggestion: tilts its head, eyes toward the message. */
  [data-mood="suggestion"] .head {
    transform: rotate(-12deg);
  }
  [data-mood="suggestion"] .look {
    transform: translate(1.1px, -0.3px);
  }

  /* Alarm: round eyes, trembling. */
  @keyframes tremble {
    0%,
    100% {
      transform: translateX(0);
    }
    25% {
      transform: translateX(-0.7px);
    }
    75% {
      transform: translateX(0.7px);
    }
  }
  [data-mood="alarm"] .head {
    animation: tremble 0.16s linear infinite;
  }

  /* Asleep: slow breathing. */
  @keyframes doze {
    0%,
    100% {
      transform: translateY(0) scale(1);
    }
    50% {
      transform: translateY(0.6px) scale(1.02, 0.97);
    }
  }
  [data-mood="sleep"] .head {
    animation: doze 3.2s ease-in-out infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .face,
    .face * {
      animation: none !important;
      transition: none !important;
    }
  }
</style>
