// The third kind of platform capability: a stream with a subscription.
//
// Location Kit measures where you are (that's the platform's job: GNSS, wifi,
// cells). The core turns successive fixes into what a person actually asks:
// how far have I walked, in which direction, how fast.
//
// The track reads as four figures on a 2×2 grid — walked, speed, fixes, time —
// with the raw fix underneath as meta, because the figures are the answer and
// the coordinates are the evidence.

import { abilityAccessCtrl, common, Permissions } from '@kit.AbilityKit';
import { geoLocationManager } from '@kit.LocationKit';
import { backgroundTaskManager } from '@kit.BackgroundTasksKit';
import { wantAgent } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { GeoPoint, rustcore } from '../../core/RustCore';
import { AppLife, appLife } from '../../state/AppLife';

@ComponentV2
export struct LocationCard {
  @Local private status: string = 'not tracking';
  @Local private point: GeoPoint | undefined = undefined;
  // @Local, or the button's label won't follow: a plain field changes the
  // value and leaves the UI showing the old one. (V2 keeps its own set of
  // decorators — @State belongs to the V1 world and can't be mixed in.)
  @Local private watching: boolean = false;
  /**
   * A continuous task is running: the system keeps this process alive in the
   * background (and shows a notification saying so).
   */
  @Local private bgTask: boolean = false;
  private listener: ((loc: geoLocationManager.Location) => void) | undefined = undefined;

  // A GPS subscription left running in the background is a battery bug: stop it
  // when the app leaves the screen, exactly like the camera.
  @Local private life: AppLife = appLife();

  @Monitor('life.foreground')
  onForegroundChanged(): void {
    if (!this.life.foreground) {
      this.onBackgrounded();
    }
  }

  aboutToDisappear(): void {
    this.stop();
  }

  private onBackgrounded(): void {
    if (!this.watching) {
      return;
    }
    if (this.bgTask) {
      // The continuous task is exactly what earns the right to keep the GPS
      // subscription alive here.
      this.status = 'tracking in the background';
      return;
    }
    this.stop();
    this.status = 'not tracking (backgrounded)';
  }

  /** Coarse and precise together: asking for LOCATION alone is refused. */
  private async askPermission(): Promise<boolean> {
    const permissions: Permissions[] = [
      'ohos.permission.APPROXIMATELY_LOCATION',
      'ohos.permission.LOCATION',
    ];
    try {
      const at = abilityAccessCtrl.createAtManager();
      const result = await at.requestPermissionsFromUser(
        getContext(this) as common.UIAbilityContext, permissions);
      if (result.authResults.length < 2 || result.authResults[0] !== 0) {
        this.status = 'location permission denied';
        return false;
      }
      return true;
    } catch (err) {
      this.status = `permission: ${(err as BusinessError).message}`;
      return false;
    }
  }

  private async start(): Promise<void> {
    if (!await this.askPermission()) {
      return;
    }
    const request: geoLocationManager.ContinuousLocationRequest = {
      interval: 1,
      locationScenario: geoLocationManager.PowerConsumptionScenario.HIGH_POWER_CONSUMPTION,
    };
    const listener = (loc: geoLocationManager.Location): void => {
      // One line of ArkTS per fix: the core does the rest.
      const result = rustcore.location(loc.latitude, loc.longitude, loc.accuracy);
      if (!result.ok || result.point === undefined) {
        this.status = result.error ?? 'the core could not use that fix';
        return;
      }
      this.point = result.point;
      // The figures below are the readout now, so the footer goes quiet: it is
      // for what went wrong, not for repeating the coordinates.
      this.status = '';
    };
    this.listener = listener;
    rustcore.locationReset();
    geoLocationManager.on('locationChange', request, listener);
    this.watching = true;
    this.status = 'waiting for a fix… (indoors, this can take a while)';
  }

  /**
   * A continuous task: the app asks the system for permission to stay alive
   * off-screen, and gets a notification in return (the user can see, and stop,
   * what is running). Declared in module.json5 as backgroundModes: ["location"].
   */
  private async startBackground(): Promise<void> {
    const ctx = getContext(this) as common.UIAbilityContext;
    try {
      const info: wantAgent.WantAgentInfo = {
        wants: [{
          bundleName: ctx.abilityInfo.bundleName,
          abilityName: ctx.abilityInfo.name,
        }],
        actionType: wantAgent.OperationType.START_ABILITY,
        requestCode: 0,
        wantAgentFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG],
      };
      const agent = await wantAgent.getWantAgent(info);
      await backgroundTaskManager.startBackgroundRunning(ctx, ['location'], agent);
      this.bgTask = true;
      this.status = 'tracking · continuous task on';
    } catch (err) {
      this.status = `background: ${(err as BusinessError).message}`;
    }
  }

  private async stopBackground(): Promise<void> {
    const ctx = getContext(this) as common.UIAbilityContext;
    try {
      await backgroundTaskManager.stopBackgroundRunning(ctx);
    } catch (err) {
      // Stopping something that already ended is not an error worth the UI.
    }
    this.bgTask = false;
  }

  private stop(): void {
    if (!this.watching) {
      return;
    }
    this.watching = false;
    if (this.listener !== undefined) {
      geoLocationManager.off('locationChange', this.listener);
      this.listener = undefined;
    }
    if (this.bgTask) {
      this.stopBackground();
    }
    rustcore.locationReset();
    this.point = undefined;   // the track is gone: don't leave its numbers behind
    this.status = 'not tracking';
  }

  /** Standing still has no direction: a 0° reading there is atan2(0, 0). */
  private bearingLine(p: GeoPoint): string {
    return p.moved > 1 ? ` · bearing ${p.bearing.toFixed(0)}°` : '';
  }

  // The figures read this.point in build() themselves. They used to arrive as
  // builder parameters, which are passed by value — the numbers then stayed at
  // whatever they were when the panel was first drawn.
  private walkedLine(): string {
    const p = this.point;
    return p === undefined ? '—' : `${p.moved.toFixed(0)} m`;
  }

  private speedLine(): string {
    const p = this.point;
    return p === undefined ? '—' : `${p.speed.toFixed(1)} m/s`;
  }

  private fixesLine(): string {
    const p = this.point;
    return p === undefined ? '—' : `${p.fixes}`;
  }

  private timeLine(): string {
    const p = this.point;
    return p === undefined ? '—' : `${p.seconds.toFixed(0)} s`;
  }

  private coordLine(): string {
    const p = this.point;
    if (p === undefined) {
      return '';
    }
    return `${p.lat.toFixed(5)}, ${p.lon.toFixed(5)} · ±${p.accuracy.toFixed(0)} m${this.bearingLine(p)}`;
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('Location')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        if (this.watching) {
          // A second, quieter action: it only exists while a track is running,
          // and it changes how the track survives, not what it measures.
          Button(this.bgTask ? 'Stop background' : 'Keep in background')
            .type(ButtonType.Normal)
            .height(32)
            .fontSize(13)
            .borderRadius(8)
            .padding({ left: 12, right: 12 })
            .backgroundColor($r('app.color.surface_raised'))
            .fontColor($r('app.color.text_primary'))
            .onClick(() => {
              if (this.bgTask) {
                this.stopBackground();
                this.status = 'tracking (foreground only)';
              } else {
                this.startBackground();
              }
            })
        }
        Button(this.watching ? 'Stop' : 'Track')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 16, right: 16 })
          .backgroundColor(this.watching ? $r('app.color.surface_raised') : $r('app.color.accent_tint'))
          .fontColor(this.watching ? $r('app.color.text_primary') : $r('app.color.accent'))
          .onClick(() => {
            if (this.watching) {
              this.stop();
            } else {
              this.start();
            }
          })
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      if (this.point !== undefined) {
        Row({ space: 16 }) {
          Column({ space: 16 }) {
            Column({ space: 4 }) {
              Text('Walked')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
              Text(this.walkedLine())
                .fontSize(20)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
            .alignItems(HorizontalAlign.Start)
            Column({ space: 4 }) {
              Text('Fixes')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
              Text(this.fixesLine())
                .fontSize(20)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
            .alignItems(HorizontalAlign.Start)
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)

          Column({ space: 16 }) {
            Column({ space: 4 }) {
              Text('Speed')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
              Text(this.speedLine())
                .fontSize(20)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
            .alignItems(HorizontalAlign.Start)
            Column({ space: 4 }) {
              Text('Time')
                .fontSize(11)
                .fontColor($r('app.color.text_muted'))
              Text(this.timeLine())
                .fontSize(20)
                .fontColor($r('app.color.text_primary'))
                .fontFeature('"tnum" 1')
            }
            .alignItems(HorizontalAlign.Start)
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)
        }
        .width('100%')
        .alignItems(VerticalAlign.Top)

        Text(this.coordLine())
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
          .width('100%')
      }

      if (this.status.length > 0) {
        Text(this.status)
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
