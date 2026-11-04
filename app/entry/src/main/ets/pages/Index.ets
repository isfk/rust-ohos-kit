// The page as assembly: layout, and the two pieces of state that are genuinely
// page-level (the todos, and how much room the floating system bars take).
// Everything else lives with the feature that uses it — the camera with the
// camera, the sensor with the level, each measurement with its own card.
//
// The rules this follows are in AGENTS.md ("边界"): state stays in the component
// that uses it, a Go event has exactly one entry point, and nothing here talks to
// librustcore.so directly.
//
// Visually it is one column of panels on a 4px rhythm: 16 between panels, 12
// inside one. Each panel owns its header row, so grouping is carried by the
// surface rather than by a rule between unrelated controls.

import { window, LengthMetrics } from '@kit.ArkUI';
import { rustcore, Event, Todo } from '../core/RustCore';
import { onCoreEvent, startCoreEvents } from '../core/events';
import { BridgeCard } from '../features/bench/BridgeCard';
import { DbCard } from '../features/bench/DbCard';
import { MeasureCard } from '../features/bench/MeasureCard';
import { CameraCard } from '../features/camera/CameraCard';
import { Heartbeat } from '../features/heartbeat/Heartbeat';
import { LevelRow } from '../features/level/LevelRow';
import { LocationCard } from '../features/location/LocationCard';
import { PhotoCard } from '../features/photo/PhotoCard';
import { TodoComposer } from '../features/todos/TodoComposer';
import { TodoList } from '../features/todos/TodoList';

@Entry
@ComponentV2
struct Index {
  /** The list: this page owns it, and only this page writes it. */
  @Local todos: Todo[] = [];
  /** Is the page on screen? What the accelerometer uses to decide whether to run. */
  @Local active: boolean = false;
  /**
   * Room the floating system bars take. The window is full screen (see
   * EntryAbility), so the page leaves that room itself — and it asks the window
   * for the number rather than guessing, because a notch and a gesture bar are
   * different heights on different phones. The defaults are this phone's, so the
   * first frame doesn't sit under the clock.
   */
  @Local topInset: number = 44;
  @Local bottomInset: number = 28;
  private offEvents: Array<() => void> = [];

  aboutToAppear(): void {
    // Subscribe before the core starts: it pushes its first list as soon as it
    // has read one, and an event with nobody listening is an event lost.
    this.offEvents = [
      onCoreEvent('todos', (e: Event): void => {
        this.todos = e.todos ?? [];
      }),
    ];
    startCoreEvents();
    rustcore.start(getContext(this).filesDir);

    window.getLastWindow(getContext(this)).then((win: window.Window) => {
      const status = win.getWindowAvoidArea(window.AvoidAreaType.TYPE_SYSTEM);
      const gesture = win.getWindowAvoidArea(window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR);
      // getUIContext().px2vp, not the global: the free function is deprecated.
      this.topInset = this.getUIContext().px2vp(status.topRect.height);
      this.bottomInset = this.getUIContext().px2vp(gesture.bottomRect.height);
    }).catch(() => {
      // No window yet: the defaults keep the content clear of the bars anyway.
    });
  }

  onPageShow(): void {
    this.active = true;
    rustcore.lifecycle(true);
  }

  onPageHide(): void {
    this.active = false;
    rustcore.lifecycle(false);
  }

  aboutToDisappear(): void {
    this.offEvents.forEach((off: () => void): void => {
      off();
    });
    this.offEvents = [];
    rustcore.stop();
  }

  // The core answers with the list after each change, so there is nothing to
  // re-read and no second source of truth.
  private add(title: string): void {
    this.todos = rustcore.add(title);
  }

  private toggle(id: number): void {
    this.todos = rustcore.toggle(id);
  }

  private remove(id: number): void {
    this.todos = rustcore.remove(id);
  }

  /** The panel's own count. It is page chrome now, so the page counts it. */
  private leftLine(): string {
    if (this.todos.length === 0) {
      return 'empty';
    }
    const left = this.todos.filter((t: Todo): boolean => !t.done).length;
    return `${left} of ${this.todos.length} left`;
  }

  build() {
    // The whole page scrolls: the panels, the preview and the list are taller
    // than any phone. The camera surface rides along inside it — verified on the
    // device: it scrolls and it survives a stop/start without coming back the
    // wrong size.
    Scroll() {
      Column({ space: 16 }) {
        // The page header: what this is, and the core's own clock. The uptime
        // belongs here and not in a card — it is about the whole page.
        Row({ space: 12 }) {
          Column({ space: 4 }) {
            Text('RustKit')
              .fontSize(28)
              .fontWeight(FontWeight.Medium)
              .fontColor($r('app.color.text_primary'))
            Text('The logic in Rust, the interface in ArkTS')
              .fontSize(13)
              .fontColor($r('app.color.text_secondary'))
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)

          Heartbeat()
        }
        .width('100%')
        .alignItems(VerticalAlign.Top)

        // Todos: the field and the list are one panel, so the list a row is
        // added to is the thing directly under the field that adds it.
        Column({ space: 12 }) {
          Row({ space: 12 }) {
            Text('Todos')
              .fontSize(15)
              .fontWeight(FontWeight.Medium)
              .fontColor($r('app.color.text_primary'))
            Blank()
            Text(this.leftLine())
              .fontSize(12)
              .fontColor($r('app.color.text_muted'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
          .alignItems(VerticalAlign.Center)

          TodoComposer({
            onAdd: (title: string): void => {
              this.add(title);
            },
          })

          TodoList({
            todos: this.todos,
            onToggle: (id: number): void => {
              this.toggle(id);
            },
            onRemove: (id: number): void => {
              this.remove(id);
            },
          })
        }
        .alignItems(HorizontalAlign.Start)
        .width('100%')
        .padding(16)
        .backgroundColor($r('app.color.surface'))
        .borderRadius(12)

        // A photo the platform takes and decodes, and the core measures.
        PhotoCard()

        // A platform stream with a subscription: where you are, and how far
        // you've walked since the first fix.
        LocationCard()

        // A platform callback at 50 Hz, into Go, and back out smoothed.
        LevelRow({ active: this.active })

        // A runtime permission, a native surface, and a session to tear down.
        CameraCard()

        // Three measurements, each with its own panel: the wire format, the
        // bridge, and the database. Measured rather than argued about. The
        // label is closer to its own panels than to the one above it.
        Text('Measurements')
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .padding({ top: 8 })

        MeasureCard()
        BridgeCard()
        DbCard()
      }
      .width('100%')
      .alignItems(HorizontalAlign.Start)
      .padding({
        top: LengthMetrics.vp(this.topInset + 12),
        start: LengthMetrics.vp(16),
        end: LengthMetrics.vp(16),
        bottom: LengthMetrics.vp(this.bottomInset + 16),
      })
    }
    .width('100%')
    .height('100%')
    .backgroundColor($r('app.color.page_bg'))
    .expandSafeArea([SafeAreaType.SYSTEM], [SafeAreaEdge.TOP, SafeAreaEdge.BOTTOM])
  }
}
