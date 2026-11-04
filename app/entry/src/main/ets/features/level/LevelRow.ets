// The accelerometer, end to end: the platform's 50 Hz callback goes straight into
// Go, comes back smoothed and turned into degrees, and is drawn as a level.
//
// The subscription and the sensor belong to this component, not to the page:
// a page that owns a 50 Hz stream starts bleeding responsibilities. The two
// things that decide whether it runs are @Param active (the page is on screen)
// and AppLife (the app is in the foreground) — the same pattern LocationCard
// uses, and the reason Home doesn't leave the accelerometer running.
//
// The face is a real instrument's: an outer ring, a crosshair, an inner ring the
// bubble travels in, and a bubble clamped at that ring's edge the way a spirit
// level's is. It is the one element on the page that is worth the bright accent.

import { sensor } from '@kit.SensorServiceKit';
import { Event, rustcore, LevelReading } from '../../core/RustCore';
import { onCoreEvent } from '../../core/events';
import { AppLife, appLife } from '../../state/AppLife';

/** The bubble stops here: the inner ring's radius minus the bubble's. */
const BUBBLE_LIMIT = 34;

@ComponentV2
export struct LevelRow {
  /** The page is showing. Index sets it from onPageShow/onPageHide. */
  @Param active: boolean = false;
  @Local private level: LevelReading | undefined = undefined;
  @Local private error: string = '';
  /** Ability-level foreground, which is what catches the Home key. */
  @Local private life: AppLife = appLife();
  private off: () => void = (): void => {};
  private running: boolean = false;

  aboutToAppear(): void {
    this.off = onCoreEvent('level', (e: Event): void => {
      this.level = e.level;
    });
    this.sync();
  }

  aboutToDisappear(): void {
    this.off();
    this.stop();
  }

  @Monitor('active')
  onActiveChanged(): void {
    this.sync();
  }

  @Monitor('life.foreground')
  onForegroundChanged(): void {
    this.sync();
  }

  private sync(): void {
    if (this.active && this.life.foreground) {
      this.start();
    } else {
      this.stop();
    }
  }

  /**
   * One callback per sample, 20 ms apart, into Go. The bridge passes three
   * doubles and allocates nothing; the core decides how often the UI hears back.
   */
  private onAccel = (data: sensor.AccelerometerResponse): void => {
    rustcore.accel(data.x, data.y, data.z);
  };

  private start(): void {
    if (this.running) {
      return;
    }
    try {
      sensor.on(sensor.SensorId.ACCELEROMETER, this.onAccel, { interval: 20000000 }); // 20ms: 50Hz
      this.running = true;
      this.error = '';
    } catch (err) {
      this.error = `${err}`;
    }
  }

  private stop(): void {
    if (!this.running) {
      return;
    }
    try {
      sensor.off(sensor.SensorId.ACCELEROMETER, this.onAccel);
    } catch (err) {
      // already off
    }
    this.running = false;
  }

  /** Where the bubble sits: roll to the right, pitch up, both clamped. */
  private bubbleX(): number {
    const l = this.level;
    if (l === undefined) {
      return 0;
    }
    return Math.max(-BUBBLE_LIMIT, Math.min(BUBBLE_LIMIT, l.roll));
  }

  private bubbleY(): number {
    const l = this.level;
    if (l === undefined) {
      return 0;
    }
    return Math.max(-BUBBLE_LIMIT, Math.min(BUBBLE_LIMIT, -l.pitch));
  }

  private rateLine(): string {
    const l = this.level;
    return l === undefined ? '' : `${l.rate.toFixed(0)} samples/s`;
  }

  /** The footer says why there are no numbers yet, and nothing once there are. */
  private noteLine(): string {
    if (this.error.length > 0) {
      return this.error;
    }
    return this.level === undefined ? 'waiting for the accelerometer…' : '';
  }

  private signed(v: number): string {
    return (v >= 0 ? '+' : '') + v.toFixed(1);
  }

  private pitchLine(): string {
    const l = this.level;
    return l === undefined ? '—' : `${this.signed(l.pitch)}°`;
  }

  private rollLine(): string {
    const l = this.level;
    return l === undefined ? '—' : `${this.signed(l.roll)}°`;
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 12 }) {
        Text('Level')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        Text(this.rateLine())
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      Row({ space: 20 }) {
        // In build() and not in a builder: a builder's parameters are passed by
        // value, so the bubble and the degrees would have sat on their first
        // sample forever.
        Stack({ alignContent: Alignment.Center }) {
          Circle()
            .width(120)
            .height(120)
            .fill($r('app.color.surface_raised'))
            .stroke($r('app.color.hairline'))
            .strokeWidth(1)
          // The crosshair, then the ring the bubble travels in: hairlines only,
          // so the bubble is the only thing on the face with weight.
          Row()
            .width(120)
            .height(1)
            .backgroundColor($r('app.color.hairline'))
          Row()
            .width(1)
            .height(120)
            .backgroundColor($r('app.color.hairline'))
          Circle()
            .width(96)
            .height(96)
            .fill('#00000000')
            .stroke($r('app.color.divider'))
            .strokeWidth(1)
          Circle()
            .width(24)
            .height(24)
            .fill($r('app.color.accent_live'))
            .translate({ x: this.bubbleX(), y: this.bubbleY() })
        }
        .width(120)
        .height(120)
          Column({ space: 12 }) {
            Row({ space: 8 }) {
              Text('pitch')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
                .width(32)
              Text(this.pitchLine())
                .fontSize(15)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
            Row({ space: 8 }) {
              Text('roll')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
                .width(32)
              Text(this.rollLine())
                .fontSize(15)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      if (this.noteLine().length > 0) {
        Text(this.noteLine())
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .maxLines(2)
          .width('100%')
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding(16)
    .backgroundColor($r('app.color.surface'))
    .borderRadius(12)
  }
}
