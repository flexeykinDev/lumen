// Translation for the settings window.
//
// The English string *is* the key. A key like `general.startWithWindows.label`
// means every piece of prose lives away from where it is read, and the code
// stops saying what it puts on screen; with the English inline, an untranslated
// string still renders correctly in English instead of showing an identifier to
// the user. The only ids are the long prose blocks, where inlining the whole
// paragraph twice would bury the markup.
//
// Anything missing from the table falls through to English. That is the
// deliberate failure mode: a half-translated panel is usable, a panel full of
// `hint.hotkeys` is not.

export type Language = "auto" | "en" | "ru";

/** English → Russian, for labels, descriptions and short controls. */
const RU: Record<string, string> = {
  // First-run tour
  "Skip the tour": "Пропустить",
  "Not now": "Не сейчас",
  "Turn it on": "Включить",
  "Keep it on": "Оставить включённым",
  "Open settings": "Открыть настройки",
  Done: "Готово",
  "Volume, from the taskbar": "Громкость с панели задач",
  "Scroll anywhere over the taskbar to change volume. Over an app's button it moves that app alone — which is the one that reaches a stream or a recording, because the master slider is applied after your audio has already been captured.":
    "Прокрутите колесо над панелью задач, чтобы изменить громкость. Над кнопкой приложения меняется громкость только этого приложения — именно она слышна на стриме и в записи, потому что общий регулятор применяется уже после захвата звука.",
  "Close an app from the taskbar": "Закрытие приложения с панели задач",
  "Middle-click an app's taskbar button to close it, without finding its window first. It sends the same request the X button does, so nothing is forced and unsaved work still prompts.":
    "Средний клик по кнопке на панели задач закрывает приложение, не разыскивая его окно. Отправляется тот же запрос, что и по кнопке «×»: ничего не убивается принудительно, а несохранённая работа по-прежнему спросит.",
  "Synced lyrics under the track, following the music line by line.":
    "Синхронизированный текст под треком, строка за строкой.",
  "This is the only feature that uses the network: it sends the artist, title and album of what you play to lrclib.net.":
    "Единственная функция, которая использует сеть: исполнитель, название и альбом отправляются на lrclib.net.",
  "Put the capsule where you want it": "Поставьте капсулу, куда хотите",
  "It sits above the taskbar by default. Drag it anywhere on screen — near a corner it snaps to that corner and stays there; dropped in open space it stays exactly where you let go. Middle-click hides it until the next track.":
    "По умолчанию она стоит над панелью задач. Перетащите её куда угодно: рядом с углом она примагнитится к этому углу и останется там, а отпущенная на свободном месте останется ровно там, где вы её отпустили. Средний клик скрывает её до следующего трека.",
  "Louder than 100%": "Громче 100%",
  "Windows caps every volume control at 100%. Lumen can go past it, and add bass, by processing the sound itself.":
    "Windows ограничивает любую громкость 100%. Lumen может выйти за предел и добавить басов, обрабатывая звук самостоятельно.",
  "It captures the playing app, turns it right down, and plays a boosted copy instead — about 30 ms of delay and some CPU while it runs. Easy to turn on later in Audio.":
    "Захватывает играющее приложение, сильно убавляет его и вместо него воспроизводит усиленную копию — около 30 мс задержки и немного процессора, пока работает. Позже можно включить в разделе «Звук».",
  "Lumen sits in the tray using no CPU when nothing is playing. Starting it with Windows means it is simply always there.":
    "Lumen живёт в трее и не тратит процессор, пока ничего не играет. С автозапуском он просто всегда на месте.",
  "That's everything": "Это всё",
  "The capsule appears above the taskbar when something plays. Middle-click it to hide it, drag it anywhere, and everything here — plus hotkeys, Discord and the rest — is in Settings whenever you want it.":
    "Капсула появляется над панелью задач, когда что-то играет. Средний клик скрывает её, перетаскивание переносит куда угодно, а всё это — плюс горячие клавиши, Discord и остальное — есть в настройках.",

  // Chrome
  Saved: "Сохранено",
  Close: "Закрыть",
  Restart: "Перезапуск",
  "Restart Lumen": "Перезапустить Lumen",
  "Loading settings…": "Загрузка настроек…",

  // Tabs and headings
  General: "Основное",
  Appearance: "Внешний вид",
  "Audio & mouse": "Звук и мышь",
  Hotkeys: "Горячие клавиши",
  Discord: "Discord",
  Lyrics: "Текст песни",
  Advanced: "Дополнительно",
  About: "О программе",
  "What it shows": "Что показывать",
  "Album cover": "Обложка альбома",
  Buttons: "Кнопки",
  Players: "Плееры",

  // General
  Language: "Язык",
  "Follows Windows unless you pick one.": "По умолчанию — язык Windows.",
  "Start with Windows": "Запускать вместе с Windows",
  "Adds Lumen to this user's startup entries. The path is re-checked at every launch, so moving the exe does not silently break it.":
    "Добавляет Lumen в автозапуск текущего пользователя. Путь проверяется при каждом запуске, поэтому перемещение файла не сломает автозапуск незаметно.",
  "Stay on screen while paused": "Оставаться на экране на паузе",
  "Off hides the capsule until playback resumes.":
    "Если выключено, капсула скрывается до возобновления воспроизведения.",
  "Open the panel on a track change": "Раскрывать панель при смене трека",
  "Briefly expands to show the new track, then collapses again.":
    "Ненадолго раскрывается, показывает новый трек и сворачивается обратно.",
  "How long it stays open": "Сколько остаётся раскрытой",
  "Pause when the machine locks": "Ставить на паузу при блокировке",
  "Only ever resumes what it paused itself, and only if nothing else started playing while you were away.":
    "Возобновляет только то, что поставил на паузу сам, и только если за время отсутствия ничего другого не запускалось.",
  "Start it playing again on unlock": "Возобновлять после разблокировки",

  // Appearance
  Position: "Положение",
  "Where the capsule sits when it is not being dragged.":
    "Где находится капсула, пока её не перетаскивают.",
  "Above the taskbar": "Над панелью задач",
  "Bottom left": "Снизу слева",
  "Bottom right": "Снизу справа",
  "Top left": "Сверху слева",
  "Top right": "Сверху справа",
  "Wherever I dropped it": "Там, где оставили",
  "discord.com/developers → your application → Application ID.":
    "discord.com/developers → ваше приложение → Application ID.",
  "Show as": "Показывать как",
  "Listening to — with progress": "«Слушает» — с полосой прогресса",
  "Playing — with buttons": "«Играет» — с кнопками",
  "Listening gives you the progress bar, Playing gives you the buttons. Discord does not draw buttons on a Listening activity, so this is the choice between them.":
    "«Слушает» даёт полосу прогресса, «Играет» — кнопки. Discord не рисует кнопки у активности типа «Слушает», поэтому приходится выбирать одно из двух.",
  Sync: "Синхронизация",
  "Shifts every line. Drag it while the song plays: left if the words come late, right if they run ahead.":
    "Сдвигает все строки. Тяните прямо во время песни: влево — если слова опаздывают, вправо — если забегают вперёд.",
  "On top of the sync above, for guessed timings only — they drift by their nature, and a real synced lyric should not be corrected twice.":
    "Поверх синхронизации выше и только для рассчитанных таймингов: они плывут по своей природе, а настоящий синхронизированный текст не нужно поправлять дважды.",
  "Interface size": "Размер интерфейса",
  "For a 2K or 4K screen running at 100%, where everything correctly sized is also tiny. Scales the capsule and this window together.":
    "Для экранов 2K и 4K при масштабе 100%, где всё правильного размера выглядит крошечным. Масштабирует капсулу и это окно вместе.",
  Backdrop: "Подложка",
  "Acrylic samples what is actually behind the window; Mica only tints from the wallpaper.":
    "Acrylic показывает то, что реально за окном; Mica берёт оттенок только у обоев.",
  Auto: "Автоматически",
  Corners: "Углы",
  Rounded: "Скруглённые",
  Square: "Прямые",
  Monitor: "Монитор",
  "Which display to dock to when several are attached.":
    "К какому экрану прижиматься, если их несколько.",
  Primary: "Основной",
  "Wherever the pointer is": "Там, где курсор",
  Spectrum: "Спектр",
  Theme: "Тема",
  Dark: "Тёмная",
  Light: "Светлая",
  System: "Как в системе",

  // Audio & mouse
  Boost: "Усиление",
  "Wheel and clicks": "Колесо и клики",
  "Volume boost": "Усиление громкости",
  "Past the 100% Windows allows. A limiter keeps peaks from clipping, so the loudest parts compress rather than distort.":
    "Выше 100%, которые разрешает Windows. Лимитер не даёт пикам срезаться: самые громкие места сжимаются, а не хрипят.",
  Loudness: "Громкость",
  "Bass boost": "Усиление басов",
  "A shelf below 120 Hz, so it lifts weight without muddying voices.":
    "Полка ниже 120 Гц: добавляет веса, не превращая голоса в кашу.",
  "Volume step": "Шаг громкости",
  "How far one wheel notch moves the level.":
    "На сколько один щелчок колеса меняет громкость.",
  "Scroll the taskbar to change volume": "Прокрутка панели задач меняет громкость",
  "Over an app's button it changes that app's own volume; over an empty stretch it moves the system master.":
    "Над кнопкой приложения меняется громкость этого приложения; над пустым местом — общая громкость системы.",
  "Per-app volume": "Громкость отдельного приложения",
  "The app's own level rather than the system master. This is the one that reaches a stream: the master is applied after anything capturing your audio has already taken it.":
    "Громкость самого приложения, а не общая. Именно она слышна на стриме: общая применяется уже после того, как звук захвачен для трансляции.",
  "Work over full-screen windows": "Работать поверх полноэкранных окон",
  "Keeps the taskbar wheel working where the bar would be when a game is covering it, targeting the game.":
    "Колесо продолжает работать там, где была бы панель задач, когда её закрывает игра — громкость меняется у самой игры.",
  "Close an app by clicking its taskbar button": "Закрывать приложение кликом по кнопке в панели задач",
  "Sends a close request, which the app can still refuse or prompt about. Right-click would replace the jump list.":
    "Отправляет запрос на закрытие — приложение всё ещё может отказаться или что-то спросить. Правая кнопка заменила бы список переходов.",
  Off: "Выключено",
  "Middle-click": "Средняя кнопка",
  "Right-click": "Правая кнопка",
  "Middle-click the capsule to hide it": "Средний клик по капсуле скрывает её",
  "Alt + middle-click quits Lumen": "Alt + средний клик закрывает Lumen",

  // Hotkeys
  "Previous track": "Предыдущий трек",
  "Play / pause": "Воспроизведение / пауза",
  "Next track": "Следующий трек",
  "Switch source": "Сменить источник",
  "Follow the next app that is playing.": "Переключиться на следующее играющее приложение.",

  // Discord
  "Application ID": "ID приложения",
  "Rich Presence": "Rich Presence",
  "Shows what you are listening to on your Discord profile.":
    "Показывает, что вы слушаете, в вашем профиле Discord.",
  Artist: "Исполнитель",
  "The second line. Off publishes the title alone.":
    "Вторая строка. Если выключено, публикуется только название.",
  Album: "Альбом",
  "Timestamps": "Время",
  "Keep publishing while paused": "Показывать и на паузе",
  "Show the real album cover": "Показывать настоящую обложку",
  "Label": "Подпись",
  URL: "Ссылка",

  // Lyrics
  "Show lyrics": "Показывать текст",
  "Synced lyrics follow the song, line by line.":
    "Синхронизированный текст идёт за песней, строка за строкой.",
  "Fall back to Genius": "Использовать Genius как запасной источник",
  "Nudge estimated timings": "Сдвиг рассчитанных таймингов",
  "Only affects guessed timings, never a real synced lyric. Negative shows them earlier.":
    "Влияет только на рассчитанные тайминги, не на настоящий синхронизированный текст. Отрицательное значение показывает строки раньше.",

  // Advanced
  "Gap from the edge": "Отступ от края",
  "Corner inset": "Отступ в углу",
  "Snap distance": "Дистанция примагничивания",

  // About
  "A glass music capsule for Windows 11.": "Стеклянная музыкальная капсула для Windows 11.",
  Version: "Версия",
  "Settings file": "Файл настроек",
  Mode: "Режим",
  "not persisted": "не сохраняется",
  "Portable — settings live beside the exe": "Портативный — настройки лежат рядом с exe",
  "Roaming — settings in %APPDATA%": "Roaming — настройки в %APPDATA%",
  "Acrylic samples whatever is behind the window; Mica only tints from the wallpaper.": "Acrylic показывает то, что реально за окном; Mica лишь берёт оттенок у обоев.",
  "Alt middle-click quits": "Alt + средний клик закрывает",
  "Auto expand duration": "Длительность раскрытия",
  "Between the capsule and the edge it is docked against.": "Между капсулой и краем, к которому она прижата.",
  "Boost amount": "Величина усиления",
  "Close an app from its taskbar button": "Закрытие приложения кнопкой на панели задач",
  "Close settings": "Закрыть настройки",
  "Corner shape": "Форма углов",
  "Discord animates this clock itself, so it keeps running between updates. Never shown while paused — it would count up on something that is not moving.": "Discord анимирует счётчик сам, поэтому время идёт и между обновлениями. На паузе не показывается — иначе оно шло бы у того, что стоит.",
  "Discord application ID": "ID приложения Discord",
  "Dock position": "Положение капсулы",
  "Elapsed time": "Прошедшее время",
  "Estimated lyric offset": "Сдвиг рассчитанных таймингов",
  "Follow Windows": "Как в Windows",
  "For tracks with no timed lyrics anywhere. Genius has no lyrics API, so this reads their web page and will break when it changes; the timings it produces are estimates, and are shown in italics.": "Для треков, у которых нигде нет синхронизированного текста. У Genius нет API для текстов, поэтому читается их веб-страница — она сломается при изменении вёрстки; тайминги там рассчитанные и показываются курсивом.",
  "Free position": "Свободная позиция",
  "Free position X": "Свободная позиция",
  "Free position Y": "Свободная позиция",
  "Global mouse gestures": "Глобальные жесты мыши",
  "Hover text on the large image.": "Подпись при наведении на большую обложку.",
  "How near a corner a drop must land to snap to it. Zero always keeps the exact drop position.": "Насколько близко к углу нужно отпустить капсулу, чтобы она примагнитилась. Ноль всегда оставляет её ровно там, где отпустили.",
  "Inset from the side for the corner positions.": "Отступ от края для угловых положений.",
  "Installs one system-wide low-level hook. Off means no hook is created at all, and none of the gestures below exist.": "Устанавливает один системный низкоуровневый хук. Если выключено, хук не создаётся вовсе и ни один жест ниже не работает.",
  "Keep it up while paused": "Оставлять на паузе",
  "Keep working over full-screen windows": "Работать поверх полноэкранных окон",
  "Live audio bars behind the panel. The only feature that costs CPU while it runs — about half a percent of one core, and only while the panel is open and playing.": "Живые полосы звука за панелью. Единственная функция, которая тратит процессор во время работы — около половины процента одного ядра, и только пока панель открыта и играет музыка.",
  "Middle-click hides": "Средний клик скрывает",
  "Move the app's own volume, not the master": "Менять громкость приложения, а не общую",
  "Names the app — “Listening via Lumen · Spotify” — instead of Lumen alone.": "Указывает приложение — «Listening via Lumen — Spotify» — вместо просто Lumen.",
  "Off clears the presence on a pause. A profile that still says “listening” hours after the music stopped is worse than one that says nothing.": "Если выключено, на паузе статус очищается. Профиль, который спустя часы всё ещё «слушает», хуже пустого.",
  "Over an app's button it moves that app; over an empty stretch it moves the system master.": "Над кнопкой приложения меняется его громкость; над пустым местом — общая громкость системы.",
  "Play or pause": "Воспроизведение или пауза",
  "Resume on unlock": "Возобновлять после разблокировки",
  "Scrolling where the taskbar would be still works when a game covers it, and targets the game.": "Прокрутка там, где была бы панель задач, работает и когда её закрывает игра — громкость меняется у самой игры.",
  "Sends a close request, which the app can still refuse or prompt about. Right-click replaces the jump list — choose it deliberately.": "Отправляет запрос на закрытие — приложение всё ещё может отказаться или что-то спросить. Правая кнопка заменяет список переходов, выбирайте осознанно.",
  "Show album": "Показывать альбом",
  "Show artist": "Показывать исполнителя",
  "Show elapsed time": "Показывать прошедшее время",
  "Show the player": "Показывать плеер",
  "Show while paused": "Показывать на паузе",
  "Synced lyrics follow the song, word by word.": "Синхронизированный текст идёт за песней, слово за словом.",
  "Taskbar close button": "Кнопка закрытия на панели задач",
  "Taskbar wheel volume": "Громкость колесом на панели задач",
  "The master switch for everything on this page.": "Главный выключатель для всего на этой странице.",
  "This is the one that reaches a stream: the master is applied at the endpoint, after anything capturing your audio has already taken it.": "Именно она слышна на стриме: общая применяется на устройстве вывода, уже после того, как звук захвачен для трансляции.",
  "Use the real cover art": "Использовать настоящую обложку",
  "Where a dropped capsule sits, measured from the work area's top-left. Used only by the “wherever I dropped it” position.": "Где стоит отпущенная капсула, считая от левого верхнего угла рабочей области. Используется только положением «там, где оставили».",
  "Which player": "Какой плеер",
  "Volume up": "Громкость выше",
  "The playing app's own level, same as the taskbar wheel.": "Громкость самого играющего приложения, как у колеса на панели задач.",
  "Volume down": "Громкость ниже",
  "Repeat": "Повтор",
  "Steps the player's own repeat mode: off, whole list, one track.": "Переключает режим повтора самого плеера: выключен, весь список, один трек.",
  "Hide or show the capsule": "Скрыть или показать капсулу",
  "Keep the panel open": "Держать панель открытой",
  "Toggles the setting below, so it survives a restart.": "Переключает настройку ниже, поэтому режим переживает перезапуск.",
  "Always keep the panel open": "Всегда держать панель открытой",
  "Stays expanded instead of collapsing back to the pill. It still hides when nothing is playing — there is nothing to show.": "Остаётся раскрытой вместо того, чтобы сворачиваться в пилюлю. Когда ничего не играет, всё равно скрывается — показывать нечего.",
  "Clawd": "Клод",
  "A pixel crab who dances in the capsule while music plays. Click him to make him stop, or start again.": "Пиксельный краб, который танцует в капсуле, пока играет музыка. Кликните по нему, чтобы он остановился или снова затанцевал.",
  "Show Clawd": "Показывать Клода",
  "Dance": "Танец",
  "Click him in the capsule to settle him down, or start him off again.": "Кликните по нему в капсуле, чтобы он успокоился или снова затанцевал.",
  "Bob": "Приседания",
  "Sway": "Покачивание",
  "Hop": "Прыжки",
  "Spin": "Вращение",
  "Size": "Размер",
  "Colour": "Цвет",
  "The shell. Every other tone is mixed from it, so one colour is the whole character.": "Панцирь. Все остальные оттенки смешиваются из него, поэтому один цвет задаёт всего персонажа.",
  "Accessory": "Аксессуар",
  "Nothing": "Ничего",
  "Cap": "Кепка",
  "Crown": "Корона",
  "Headphones": "Наушники",
  "Antenna": "Антенна",
  "Surprise me": "Удиви меня",
  "Source on GitHub": "Исходники на GitHub",
  "Releases": "Релизы",
  "Checking for updates…": "Проверяем обновления…",
  "is available": "уже доступна",
  "Get it": "Скачать",
  "You have the latest version.": "У вас последняя версия.",
  "Could not check for updates.": "Не удалось проверить обновления.",
  "Check again": "Проверить снова",
  "Check for updates": "Проверять обновления",
  "One request per launch to a text file in this repository. Nothing is downloaded, and nothing identifies the machine.": "Один запрос за запуск к текстовому файлу в этом репозитории. Ничего не скачивается и ничего не идентифицирует компьютер.",
  restart: "перезапуск",
};

/** The long blocks, where the English would otherwise dominate the markup. */
const PROSE: Record<string, { en: string; ru: string }> = {
  "hint.clawd": {
    en: "You found him. He lives in the capsule, dances while the music plays, and stops when you click him. None of this was here before you tapped that cover.",
    ru: "Вы его нашли. Он живёт в капсуле, танцует, пока играет музыка, и останавливается по клику. Ничего этого здесь не было, пока вы не постучали по обложке.",
  },
  "hint.hotkeys": {
    en: "Click a binding and press the keys you want. Escape cancels, Backspace clears it. These are system-wide: they work whatever has focus, and a chord another app has already claimed simply will not register.",
    ru: "Нажмите на поле и введите нужное сочетание. Escape отменяет, Backspace очищает. Клавиши глобальные: работают в любом приложении, но сочетание, уже занятое другой программой, просто не сработает.",
  },
  "notice.boost": {
    en: "Windows caps every volume control at 100%, so going louder means Lumen processing the sound itself: it captures the playing app, turns that app down to 2%, and plays a boosted copy in its place. That adds about 30 ms of delay — inaudible in music, slightly out of lip-sync in video — and costs some CPU while it runs. The app's own level is restored when boost stops or Lumen exits.",
    ru: "Windows ограничивает любую регулировку громкости 100%, поэтому громче — только обработкой звука: Lumen захватывает играющее приложение, убавляет его до 2% и вместо него воспроизводит усиленную копию. Это добавляет около 30 мс задержки (в музыке незаметно, в видео губы чуть расходятся) и тратит немного процессора, пока работает. Громкость приложения возвращается, когда усиление выключается или Lumen закрывается.",
  },
  "hint.discord": {
    en: "Presence is published as a Discord application, so it needs an application ID — that application's name is what renders after “Listening to”. Everything below decides what other people see on your profile.",
    ru: "Статус публикуется от имени приложения Discord, поэтому нужен ID приложения — его название и отображается после «Слушает». Всё, что ниже, определяет, что увидят другие в вашем профиле.",
  },
  "notice.cover": {
    en: "Discord draws the presence image from a URL or an uploaded asset, and Windows hands Lumen the cover as raw bytes — so showing the real artwork means finding the same image online. Turning this on sends the artist and title of what you play to Apple's public iTunes Search endpoint.",
    ru: "Discord берёт картинку статуса по ссылке или из загруженных ресурсов, а Windows отдаёт Lumen обложку в виде байтов — чтобы показать настоящую обложку, её нужно найти в сети. При включении исполнитель и название отправляются в публичный поиск Apple iTunes.",
  },
  "hint.buttons": {
    en: "Discord shows at most two — and, the part that catches everyone, it does not draw them on your own profile. Only other people see them.",
    ru: "Discord показывает максимум две — и, о чём все забывают, в вашем собственном профиле они не отображаются. Их видят только другие.",
  },
  "hint.players": {
    en: "Which players reach your profile. Anything not listed here is published — a newly installed app should not go missing without you knowing. Turning one off clears the presence while it plays, rather than leaving the last track frozen up there.",
    ru: "Какие плееры попадают в профиль. Всё, чего здесь нет, публикуется — только что установленное приложение не должно молча исчезнуть. Выключенный плеер очищает статус, пока играет, а не оставляет висеть последний трек.",
  },
  "notice.lyrics": {
    en: "Turning this on sends the artist, title, album and duration of what you play to lrclib.net, and to genius.com if the fallback is on.",
    ru: "При включении исполнитель, название, альбом и длительность отправляются на lrclib.net, а при включённом запасном источнике — и на genius.com.",
  },
  "hint.advanced": {
    en: "Geometry, in logical pixels — scaled by each monitor's DPI at placement time, so the numbers mean the same thing at 100% and 150%.",
    ru: "Геометрия в логических пикселях — масштабируется по DPI каждого монитора при размещении, поэтому значения означают одно и то же при 100% и 150%.",
  },
  "about.note": {
    en: "Settings save the moment you change them. Anything marked restart needs Lumen restarted, because the part of the app behind it is built once at startup.",
    ru: "Настройки сохраняются сразу. Пункты с меткой «перезапуск» требуют перезапуска Lumen: соответствующая часть приложения создаётся один раз при старте.",
  },
};

/** What Windows is set to, when the config says `auto`. */
function systemLanguage(): "en" | "ru" {
  return navigator.language.toLowerCase().startsWith("ru") ? "ru" : "en";
}

let active: "en" | "ru" = "en";

export function setLanguage(pref: Language): void {
  active = pref === "auto" ? systemLanguage() : pref;
  document.documentElement.lang = active;
}

export function currentLanguage(): "en" | "ru" {
  return active;
}

/**
 * Translate one string.
 *
 * Takes English (or a prose id) and returns what should be shown. Unknown
 * strings come back unchanged, which is why an incomplete table is safe.
 */
export function t(text: string | undefined): string {
  if (!text) return "";
  const prose = PROSE[text];
  if (prose) return active === "ru" ? prose.ru : prose.en;
  if (active === "ru") return RU[text] ?? text;
  return text;
}
