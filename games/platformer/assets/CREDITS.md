# Ассеты

`tilemap_packed.png`, `tilemap-characters_packed.png` — [Pixel Platformer 1.2](https://kenney.nl/assets/pixel-platformer), Kenney (www.kenney.nl), лицензия [CC0 1.0](http://creativecommons.org/publicdomain/zero/1.0/). Файлы из архива `kenney_pixel-platformer.zip`, папка `Tilemap/`, без изменений.

Звуки — Kenney (www.kenney.nl), [CC0 1.0](http://creativecommons.org/publicdomain/zero/1.0/), из OGG в WAV 16 бит:

| файл | пак | исходник |
|---|---|---|
| `jump.wav` | [Digital Audio](https://kenney.nl/assets/digital-audio) | `Audio/phaseJump1.ogg` |
| `coin.wav` | Digital Audio | `Audio/highUp.ogg` |
| `hurt.wav` | Digital Audio | `Audio/lowThreeTone.ogg` |
| `stomp.wav` | [Impact Sounds](https://kenney.nl/assets/impact-sounds) | `Audio/impactPunch_medium_000.ogg` |
| `win.wav` | [Music Jingles](https://kenney.nl/assets/music-jingles) | `Audio/8-Bit jingles/jingles_NES00.ogg` |

```sh
ffmpeg -i phaseJump1.ogg -c:a pcm_s16le jump.wav
```

`music.ogg` — «Level 1» из [5 Chiptunes (Action)](https://opengameart.org/content/5-chiptunes-action), Juhani Junkala, CC0; у Kenney музыкальных лупов нет. Перекодирована из WAV в 48 кГц — частоту устройства по умолчанию, при которой загрузка обходится без ресемплинга:

```sh
ffmpeg -i "Juhani Junkala [Retro Game Music Pack] Level 1.wav" -ar 48000 -c:a libvorbis -q:a 3 music.ogg
```
