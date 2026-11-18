# Builds src/skies/hlsl/neotokyo_glyphs.hlsl: the Japanese words on Neo Tokyo's signs and the
# names of the made-up brands on its adverts, as distance fields packed into the shader.
#   python neotokyo_glyphs.py <fonts folder> <out.hlsl> [check.png]
# The fonts, all under the SIL Open Font License, from github.com/google/fonts/tree/main/ofl:
#   MPLUSRounded1c-ExtraBold.ttf (mplusrounded1c), for the Japanese; its rounded strokes read like
#     bent neon tube
#   Pacifico-Regular.ttf (pacifico), Anton-Regular.ttf (anton), Orbitron[wght].ttf (orbitron),
#     Bungee-Regular.ttf (bungee), Lobster-Regular.ttf (lobster), Bangers-Regular.ttf (bangers) and
#     Righteous-Regular.ttf (righteous), one brand each
#
# A brand's name is drawn as a whole, in its own font, and cut into squares one em wide that
# follow each other like characters, so its letters keep their own widths and spacing.
#
# Each character is drawn large, its signed distance to the outline worked out, and sampled on
# a SIZE x SIZE grid over its em square: negative inside, in em units, clamped to +-RANGE and
# kept as 4 bits. 32 values fill a uint4, so with SIZE 32 each row of a character is one uint4
# and the shader reads a character's 2x2 neighbourhood in two lookups. Every lookup into the
# table costs seconds of compile time in the full sky shader, so keep SIZE at 32.
import glob
import os
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy.ndimage import distance_transform_edt, map_coordinates

SIZE, RANGE = 32, 0.08
assert SIZE == 32, 'the shader reads one row of a character per uint4'
EM, MARGIN = 256, 48

# The words, with what they say. Kept to everyday sign words.
WORDS = [
    ('東京', 'Tokyo'),
    ('ラーメン', 'ramen'),
    ('カラオケ', 'karaoke'),
    ('寿司', 'sushi'),
    ('ホテル', 'hotel'),
    ('居酒屋', 'izakaya, a pub'),
    ('焼肉', 'grilled meat'),
    ('夜', 'night'),
    ('未来', 'future'),
    ('営業中', 'open'),
    ('喫茶', 'cafe'),
    ('酒', 'sake'),
    ('麻雀', 'mahjong'),
    ('バー', 'bar'),
    ('薬', 'pharmacy'),
    ('電気', 'electric'),
    ('夢', 'dream'),
    ('光', 'light'),
    ('愛', 'love'),
    ('コンビニ', 'convenience store'),
    ('パチンコ', 'pachinko'),
    ('消費者金融', 'consumer loans'),
    ('らーめん', 'ramen, in hiragana'),
    ('ニュース', 'news'),
    ('24時間', 'open 24 hours'),
    ('新発売', 'new, on sale now'),
    ('限定', 'limited edition'),
]

# The made-up brands on the adverts: name, font, weight (for a variable font). They are
# invented, with their own lettering, so that no real brand's logo is copied.
BRANDS = [
    ('Kira Cola', 'Pacifico-Regular.ttf', None),
    ('ZAPP', 'Anton-Regular.ttf', None),
    ('HOSHI', 'Orbitron*.ttf', 900),
    ('KURO', 'Bungee-Regular.ttf', None),
    ('Sora Beer', 'Lobster-Regular.ttf', None),
    ('BIG BUN', 'Bangers-Regular.ttf', None),
    ('ONPU', 'Righteous-Regular.ttf', None),
]

# What the scrolling LED tickers say, word after word with a gap between: the news, the time
# of day the shops keep, and the shops themselves.
TICKER = ['ニュース', '東京', '24時間', '営業中', 'コンビニ', 'カラオケ', 'らーめん', '消費者金融', 'パチンコ', '居酒屋', '未来']
BLANK = 255

fonts, out = sys.argv[1], sys.argv[2]
font = ImageFont.truetype(os.path.join(fonts, 'MPLUSRounded1c-ExtraBold.ttf'), EM)
glyphs = []
for word, _ in WORDS:
    for ch in word:
        if ch not in glyphs:
            glyphs.append(ch)

canvas = EM + 2 * MARGIN
nibbles = []
for ch in glyphs:
    img = Image.new('L', (canvas, canvas), 0)
    ImageDraw.Draw(img).text((canvas / 2, canvas / 2), ch, font=font, fill=255, anchor='mm')
    inside = np.asarray(img) > 127
    # Distance to the outline in pixels, negative inside, then in em units.
    sdf = (distance_transform_edt(~inside) - distance_transform_edt(inside)) / EM
    # Sample at the centres of a SIZE x SIZE grid over the em square, row by row from the top.
    centres = MARGIN + (np.arange(SIZE) + 0.5) * (EM / SIZE)
    ys, xs = np.meshgrid(centres, centres, indexing='ij')
    grid = map_coordinates(sdf, [ys.ravel(), xs.ravel()], order=1)
    q = np.round((np.clip(grid, -RANGE, RANGE) / RANGE * 0.5 + 0.5) * 15).astype(np.uint32)
    nibbles.extend(q.tolist())


def brand_font(name, weight, size):
    f = ImageFont.truetype(glob.glob(os.path.join(fonts, name))[0], size)
    if weight:
        f.set_variation_by_axes([weight])
    return f


# Each brand's name: drawn so its letters stand 0.66 of an em tall, centred in a strip a whole
# number of ems long, and cut into squares.
brand_tiles = []
for name, font_name, weight in BRANDS:
    f = brand_font(font_name, weight, EM)
    l, t, r, b = f.getbbox(name)
    f = brand_font(font_name, weight, int(EM * 0.66 * EM / (b - t)))
    l, t, r, b = f.getbbox(name)
    count = int(np.ceil(((r - l) + (0.25 * EM)) / EM))
    strip = Image.new('L', ((count * EM) + (2 * MARGIN), EM + (2 * MARGIN)), 0)
    ImageDraw.Draw(strip).text((MARGIN + (((count * EM) - (r - l)) / 2) - l, MARGIN + ((EM - (b - t)) / 2) - t), name, font=f, fill=255)
    inside = np.asarray(strip) > 127
    sdf = (distance_transform_edt(~inside) - distance_transform_edt(inside)) / EM
    first = len(glyphs) + sum(n for _, n in brand_tiles)
    for k in range(count):
        ys, xs = np.meshgrid(MARGIN + (np.arange(SIZE) + 0.5) * (EM / SIZE), MARGIN + (k * EM) + (np.arange(SIZE) + 0.5) * (EM / SIZE), indexing='ij')
        grid = map_coordinates(sdf, [ys.ravel(), xs.ravel()], order=1)
        q = np.round((np.clip(grid, -RANGE, RANGE) / RANGE * 0.5 + 0.5) * 15).astype(np.uint32)
        nibbles.extend(q.tolist())
    brand_tiles.append((first, count))

words, start = [], 0
for word, _ in WORDS:
    words.append((start, len(word)))
    start += len(word)
text = [glyphs.index(ch) for word, _ in WORDS for ch in word]
brand_words = []
for first, count in brand_tiles:
    brand_words.append((len(text), count))
    text += list(range(first, first + count))
ticker = []
for word in TICKER:
    ticker += [glyphs.index(ch) for ch in word] + [BLANK, BLANK]

data = []
for k in range(0, len(nibbles), 8):
    v = 0
    for j, n in enumerate(nibbles[k: k + 8]):
        v |= n << (4 * j)
    data.append(v)
while len(data) % 4:
    data.append(0)

lines = []
lines.append('// Generated by skies/neotokyo_glyphs.py from M PLUS Rounded 1c ExtraBold (SIL Open Font License).')
lines.append('// Do not edit by hand; change the words there and run it again.')
lines.append('')
lines.append(f'static const uint GLYPH_SIZE = {SIZE};')
lines.append(f'static const float GLYPH_RANGE = {RANGE};')
lines.append(f'static const uint GLYPH_COUNT = {len(glyphs) + sum(n for _, n in brand_tiles)};')
lines.append(f'static const uint WORD_COUNT = {len(WORDS)};')
lines.append(f'// The long vowel mark, which turns upright in vertical writing.')
lines.append(f'static const uint GLYPH_DASH = {glyphs.index("ー")};')
lines.append('')
lines.append('// Each word: where its characters start in WORD_TEXT, and how many there are.')
lines.append('static const uint2 WORDS[%d] = {' % len(WORDS))
lines.append('\n'.join(f'    uint2({s}, {n}){"," if k + 1 < len(WORDS) else " "}   // {m}' for k, ((s, n), (w, m)) in enumerate(zip(words, WORDS))))
lines.append('};')
lines.append('')
lines.append('static const uint WORD_TEXT[%d] = { %s };' % (len(text), ', '.join(str(t) for t in text)))
lines.append('')
lines.append('// The made-up brands\' names, each drawn whole in its own font and cut into squares that')
lines.append('// follow each other like characters: where they start in WORD_TEXT, and how many squares.')
lines.append('static const uint BRAND_COUNT = %d;' % len(BRANDS))
lines.append('static const uint2 BRAND_WORDS[%d] = {' % len(BRANDS))
lines.append('\n'.join(f'    uint2({a}, {n}){"," if k + 1 < len(BRANDS) else " "}   // {BRANDS[k][0]}, in {BRANDS[k][1].split("-")[0].rstrip("*.ttf")}' for k, (a, n) in enumerate(brand_words)))
lines.append('};')
lines.append('// The words an advert uses: its slogans, and the noodles\' name.')
lines.append('static const uint WORD_NEW = %d;' % [w for w, _ in WORDS].index('新発売'))
lines.append('static const uint WORD_LIMITED = %d;' % [w for w, _ in WORDS].index('限定'))
lines.append('static const uint WORD_RAMEN = %d;' % [w for w, _ in WORDS].index('らーめん'))
lines.append('')
lines.append('// The tickers\' text, character after character; %d is a gap.' % BLANK)
lines.append('static const uint TICKER_BLANK = %d;' % BLANK)
lines.append('static const uint TICKER_LENGTH = %d;' % len(ticker))
lines.append('static const uint TICKER_TEXT[%d] = { %s };' % (len(ticker), ', '.join(str(t) for t in ticker)))
lines.append('')
lines.append(f'// {len(glyphs)} characters and {sum(n for _, n in brand_tiles)} squares of brand names, {SIZE}x{SIZE} distances of 4 bits')
lines.append('// each, 32 to a uint4, the characters in the order they first appear in the words above. This')
lines.append('// file is kept to plain ASCII for the compiler.')
lines.append('static const uint4 GLYPH_DATA[%d] = {' % (len(data) // 4))
rows = [f'    uint4({data[k]}u, {data[k + 1]}u, {data[k + 2]}u, {data[k + 3]}u)' for k in range(0, len(data), 4)]
lines.append(',\n'.join(rows))
lines.append('};')
lines.append('')
open(out, 'w', encoding='ascii', newline='\n').write('\n'.join(lines))
# With a third argument, a picture of every character as the shader will see it, for checking.
if len(sys.argv) > 3:
    d = ((np.array(nibbles, dtype=np.float32).reshape(-1, SIZE, SIZE) / 15) - 0.5) * 2 * RANGE
    big = [np.asarray(Image.fromarray(g).resize((SIZE * 4, SIZE * 4), Image.BILINEAR)) for g in d]
    big += [np.full_like(big[0], RANGE)] * (-len(big) % 12)
    sheet = np.concatenate([np.concatenate(big[r: r + 12], axis=1) for r in range(0, len(big), 12)], axis=0)
    Image.fromarray(((sheet < 0) * 255).astype(np.uint8)).save(sys.argv[3])
print(f'{len(glyphs)} characters, {sum(n for _, n in brand_tiles)} brand squares, {len(data) // 4} uint4, {len(WORDS)} words -> {out}')
