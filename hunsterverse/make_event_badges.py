#!/usr/bin/env python3
"""Draw the launcher's badges for Hunsterverse events that upstream has no art
for (src/activeEvents.js HUNSTERVERSE_EVENT_IMAGES): 64 x 64 PNGs in the
style of upstream's diamond emblems (public/extra/Tenro.png): a dark outline,
a gold rim and a coloured face with a short word, or a monster icon.

  python3 -I hunsterverse/make_event_badges.py RAVIENTE_ICON OUT_DIR

RAVIENTE_ICON: the website's assets/monsters/icons/raviente.png (64 x 64).
OUT_DIR: public/extra. Needs Pillow and the DejaVu fonts.
"""
import sys

from PIL import Image, ImageDraw, ImageFont

SIZE, SCALE = 64, 4  # drawn 4x larger, then scaled down for smooth edges
MAX_TEXT_WIDTH = 34
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
OUTLINE, RIM, RIM_DARK = (20, 14, 8, 235), (214, 172, 82, 255), (120, 86, 30, 255)

BADGES = [
    # file, face colour, lines of text (None: the monster icon)
    ("VsBracket.png", (150, 32, 32, 255), ["VS"]),
    ("RoadVote.png", (36, 96, 140, 255), ["ROAD", "VOTE"]),
    ("Campaigns.png", (150, 104, 20, 255), ["GIFT"]),
    ("Raviente.png", (226, 206, 160, 255), None),
]


def diamond(draw, inset, fill):
    s = SIZE * SCALE
    c = s / 2
    r = c - inset * SCALE
    draw.polygon([(c, c - r), (c + r, c), (c, c + r), (c - r, c)], fill=fill)


def badge(face, lines, icon):
    s = SIZE * SCALE
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    diamond(d, 1, OUTLINE)
    diamond(d, 3, RIM)
    diamond(d, 6, RIM_DARK)
    diamond(d, 7.5, face)
    if lines:
        # The largest size whose widest line fits the middle of the diamond.
        size = {1: 26, 2: 14}[len(lines)] * SCALE
        while True:
            font = ImageFont.truetype(FONT, size)
            widest = max(d.textbbox((0, 0), t, font=font)[2] - d.textbbox((0, 0), t, font=font)[0] for t in lines)
            if widest <= MAX_TEXT_WIDTH * SCALE or size <= 8 * SCALE:
                break
            size -= SCALE
        gap = 2 * SCALE
        heights = [d.textbbox((0, 0), t, font=font)[3] - d.textbbox((0, 0), t, font=font)[1] for t in lines]
        y = (s - sum(heights) - gap * (len(lines) - 1)) / 2
        for text, h in zip(lines, heights):
            box = d.textbbox((0, 0), text, font=font)
            x = (s - (box[2] - box[0])) / 2 - box[0]
            d.text((x, y - box[1]), text, font=font, fill=(255, 244, 214, 255),
                   stroke_width=SCALE, stroke_fill=(30, 18, 6, 255))
            y += h + gap
    im = im.resize((SIZE, SIZE), Image.LANCZOS)
    if icon is not None:
        art = icon.convert("RGBA").resize((40, 40), Image.LANCZOS)
        im.alpha_composite(art, (12, 12))
    return im


def main():
    icon_path, out_dir = sys.argv[1], sys.argv[2]
    for name, face, lines in BADGES:
        icon = Image.open(icon_path) if lines is None else None
        badge(face, lines, icon).save(f"{out_dir}/{name}", optimize=True)
        print(f"{out_dir}/{name}")


if __name__ == "__main__":
    main()
