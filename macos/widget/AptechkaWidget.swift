// Вид как 31.10 (c8251dc, просьба владелицы): кот (cat_ok.png) и пять строк.
// Виджет «Аптечка» для macOS 14+ (дизайн под Liquid Glass macOS 26 Tahoe).
// Аптечка (Rust) после каждой проверки пишет widget.json в контейнер этого расширения:
//   ~/Library/Containers/io.github.aptechka.widget/Data/widget.json
// Виджет читает его: кот в позе по состоянию (ok / warn / bad) и пять строк. Фон — системное стекло,
// углы скругляет система; статусы — разными значками SF Symbols (в тонированном режиме цвета пропадают).
import WidgetKit
import SwiftUI
import AppKit

struct Row: Codable, Hashable { let label: String; let status: String; let value: String }
struct State: Codable { let updated: Double; let state: String; let rows: [Row]; let lang: String? }

/// Язык: как выбран в Аптечке (поле lang в widget.json), без данных — как в системе.
/// Строки rows Аптечка пишет уже переведёнными; здесь — только свои надписи виджета.
func isRussian(_ st: State?) -> Bool {
    if let l = st?.lang { return l == "ru" }
    let first = Locale.preferredLanguages.first ?? ""
    return ["ru", "be", "kk", "ky"].contains(where: { first.hasPrefix($0) })
}
func L(_ st: State?, _ ru: String, _ en: String) -> String { isRussian(st) ? ru : en }

struct Entry: TimelineEntry { let date: Date; let st: State? }

func loadState() -> State? {
    // в песочнице расширения домашняя папка = его контейнер
    let url = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("widget.json")
    guard let d = try? Data(contentsOf: url) else { return nil }
    return try? JSONDecoder().decode(State.self, from: d)
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry(date: .now, st: nil) }
    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) { completion(Entry(date: .now, st: loadState())) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        // перечитать файл через минуту (если macOS решит экономить — обновит реже)
        completion(Timeline(entries: [Entry(date: .now, st: loadState())], policy: .after(.now.addingTimeInterval(60))))
    }
}

/// Кот из ресурсов расширения, без сглаживания (пиксель-арт).
struct Cat: View {
    let state: String
    var body: some View {
        // мордочка всегда обычная: в расширении есть только cat_ok.png
        if let url = Bundle.main.url(forResource: "cat_ok", withExtension: "png"), let img = NSImage(contentsOf: url) {
            let pic = Image(nsImage: img).resizable().interpolation(.none)
            // полноцветный кот в «тонированном» режиме виджетов (macOS 15+)
            if #available(macOS 15.0, *) {
                pic.widgetAccentedRenderingMode(.fullColor).aspectRatio(contentMode: .fit)
                    .clipShape(ContainerRelativeShape())
            } else {
                pic.aspectRatio(contentMode: .fit).clipShape(ContainerRelativeShape())
            }
        } else {
            Image(systemName: "cross.case.fill").font(.largeTitle)
        }
    }
}

func icon(_ s: String) -> (String, Color) {
    switch s {
    case "bad": return ("xmark.octagon.fill", .red)
    case "warn": return ("exclamationmark.triangle.fill", .orange)
    case "ok": return ("checkmark.circle.fill", .green)
    default: return ("circle.dashed", .secondary)
    }
}

struct RowView: View {
    let r: Row
    var body: some View {
        let (sym, col) = icon(r.status)
        HStack(spacing: 5) {
            Image(systemName: sym).foregroundStyle(col).widgetAccentable().font(.system(size: 11, weight: .semibold))
            Text(r.label).font(.system(size: 11)).foregroundStyle(.secondary).frame(width: 52, alignment: .leading)
            Text(r.value).font(.system(size: 11, weight: (r.label == "Итог" || r.label == "Total") ? .semibold : .regular)).lineLimit(1).truncationMode(.tail)
        }
    }
}

/// Данные старше 15 минут — Аптечка, похоже, закрыта: показываем это, а не прежнее «всё хорошо».
func isStale(_ st: State?, at now: Date) -> Bool {
    guard let st = st else { return false }
    return now.timeIntervalSince1970 - st.updated > 15 * 60 || st.updated > now.timeIntervalSince1970 + 15 * 60
}

struct AptechkaView: View {
    @Environment(\.widgetFamily) var family
    let e: Entry
    var body: some View {
        let stale = isStale(e.st, at: e.date)
        let st = e.st
        Group {
            if family == .systemSmall {
                VStack(spacing: 4) {
                    Cat(state: st?.state ?? "ok")
                    if stale { Text(L(st, "данные устарели", "data is stale")).font(.caption2).foregroundStyle(.secondary) }
                    else if let t = st?.rows.last { RowView(r: t) } else { Text(L(st, "Открой Аптечку", "Open First Aid Cat")).font(.caption2) }
                }
            } else {
                HStack(spacing: 10) {
                    Cat(state: st?.state ?? "ok")
                    VStack(alignment: .leading, spacing: 3) {
                        Text(L(st, "Аптечка", "First Aid Cat")).font(.system(size: 12, weight: .bold))
                        if stale { Text(L(st, "Данные устарели — открой Аптечку", "Data is stale — open the app")).font(.caption).foregroundStyle(.secondary) }
                        if let rows = st?.rows { ForEach(rows, id: \.self) { RowView(r: $0) }.opacity(stale ? 0.5 : 1) }
                        else { Text(L(st, "Открой Аптечку, чтобы она проверила систему", "Open First Aid Cat to check the system")).font(.caption).foregroundStyle(.secondary) }
                    }
                    Spacer(minLength: 0)
                }
            }
        }
        .containerBackground(for: .widget) { Color.clear }
        .widgetURL(URL(string: "aptechka://open"))
    }
}

@main
struct AptechkaWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "io.github.aptechka.widget", provider: Provider()) { AptechkaView(e: $0) }
            .configurationDisplayName(L(nil, "Аптечка", "First Aid Cat"))
            .description(L(nil, "Кот-доктор и главное о системе: сеть, VPN, ИИ, нагрузка.", "The doctor cat and the essentials of your system: network, VPN, AI, load."))
            .supportedFamilies([.systemSmall, .systemMedium])
    }
}
