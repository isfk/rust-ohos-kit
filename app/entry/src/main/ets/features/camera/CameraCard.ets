// The camera, the way a HarmonyOS app does it: the platform's preview into an
// XComponent's surface, a permission asked once, and everything released when
// the page goes away.
//
// What this proves for the Rust core + ArkUI split: a runtime permission, a
// native surface handed to a system service, and a session that must be torn
// down on a lifecycle callback — the three things a "Kit" really costs.

import { camera } from '@kit.CameraKit';
import { abilityAccessCtrl, common, Permissions } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { display } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { AppLife, appLife } from '../../state/AppLife';

const DOMAIN = 0x0000;

/**
 * The aspect of the preview *panel*. 16:9 is not vanity: an idle panel has to
 * stay short, or a 4:3 box pushes every panel below it off the first screen.
 * The frames' own aspect is separate — see `aspect`.
 */
const PANEL_ASPECT: number = 16 / 9;

@ComponentV2
export struct CameraCard {
  @Local private status: string = '';
  @Local private running: boolean = false;
  private controller: XComponentController = new XComponentController();
  private session: camera.PhotoSession | undefined = undefined;
  private surfaceId: string = '';
  private wanted: boolean = false;
  /**
   * The frames' own aspect: sizing the box with a profile's ratio squashes them.
   * 16:9 is the canvas until a profile arrives, because 4:3 made an idle panel
   * tall enough to push everything below it off the first screen.
   *
   * Held upright, the camera delivers an *upright* picture (the profile says
   * 1280×720, but the content is turned), so this value is below 1 in portrait —
   * narrower than the box, and therefore taller than it.
   */
  @Local private aspect: number = PANEL_ASPECT;

  // The page says when it's gone, and the ability says when the app goes to
  // the background: a camera left open holds the device and eats the battery,
  // so neither is optional. (aboutToDisappear alone misses the home button —
  // the page is still alive, just invisible.)
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
    if (this.session !== undefined) {
      this.stop();
      this.status = 'camera off (backgrounded)';
    }
  }

  private async start(): Promise<void> {
    this.wanted = true;
    if (this.surfaceId.length === 0) {
      this.status = 'waiting for the surface…';
      return;
    }
    await this.askPermission();
  }

  private async askPermission(): Promise<void> {
    const permissions: Permissions[] = ['ohos.permission.CAMERA'];
    const ctx = getContext(this) as common.UIAbilityContext;
    try {
      const at = abilityAccessCtrl.createAtManager();
      const result = await at.requestPermissionsFromUser(ctx, permissions);
      if (result.authResults.length === 0 || result.authResults[0] !== 0) {
        this.status = 'camera permission denied';
        this.wanted = false;
        return;
      }
    } catch (err) {
      this.status = `permission: ${(err as BusinessError).message}`;
      this.wanted = false;
      return;
    }
    this.openCamera(ctx);
  }

  /** Opens the camera and puts its preview on the XComponent's surface. */
  private openCamera(ctx: common.UIAbilityContext): void {
    try {
      const manager: camera.CameraManager = camera.getCameraManager(ctx);
      const cameras: camera.CameraDevice[] = manager.getSupportedCameras();
      const back: camera.CameraDevice | undefined = cameras.find(
        (c: camera.CameraDevice) => c.cameraPosition === camera.CameraPosition.CAMERA_POSITION_BACK);
      if (back === undefined) {
        this.status = 'no back camera';
        return;
      }
      const capability: camera.CameraOutputCapability =
        manager.getSupportedOutputCapability(back, camera.SceneMode.NORMAL_PHOTO);
      if (capability.previewProfiles.length === 0) {
        this.status = 'no preview profile';
        return;
      }
      const input: camera.CameraInput = manager.createCameraInput(back);
      // A preview-sized profile: the camera's largest is wasted on a panel
      // this small and costs power, so take the biggest at or below 1280.
      let profile: camera.Profile = capability.previewProfiles[0];
      for (const p of capability.previewProfiles) {
        if (p.size.width <= 1280 && p.size.width > profile.size.width) {
          profile = p;
        }
      }
      // Every preview profile is landscape (1280×720 and friends), but the
      // camera rotates the picture to match the *screen* before handing it to
      // this surface: held upright, what arrives is a portrait frame. Sizing the
      // canvas from the profile's ratio stretched the picture ~3x sideways
      // (measured), so the ratio has to come from the screen, not the profile.
      const screen = display.getDefaultDisplaySync();
      const portrait: boolean = screen.width < screen.height;
      this.aspect = portrait ?
        profile.size.height / profile.size.width :
        profile.size.width / profile.size.height;
      hilog.info(DOMAIN, 'camera', 'profile %{public}dx%{public}d screen %{public}dx%{public}d aspect %{public}s',
        profile.size.width, profile.size.height, screen.width, screen.height, this.aspect.toFixed(4));
      const preview: camera.PreviewOutput = manager.createPreviewOutput(profile, this.surfaceId);
      const session: camera.PhotoSession = manager.createSession<camera.PhotoSession>(camera.SceneMode.NORMAL_PHOTO);

      input.open().then(() => {
        session.beginConfig();
        session.addInput(input);
        session.addOutput(preview);
        session.commitConfig();
        // start() isn't a promise in every version of this API (a .then on a
        // void silently threw, and the UI never learned it was live): take the
        // session as running, and let a real failure say otherwise.
        session.start();
        this.session = session;
        this.running = true;
        this.status = `${profile.size.width}×${profile.size.height} preview`;
      }).catch((err: BusinessError) => {
        this.status = `open: ${err.message}`;
      });
    } catch (err) {
      this.status = `${(err as BusinessError).message}`;
    }
  }

  private stop(): void {
    this.wanted = false;
    const session = this.session;
    this.session = undefined;
    this.running = false;
    if (session === undefined) {
      return;
    }
    try {
      session.stop();
      session.release();
      this.status = 'camera off';
    } catch (err) {
      this.status = `stop: ${(err as BusinessError).message}`;
    }
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('Camera')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        Button(this.running ? 'Stop' : 'Start')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 16, right: 16 })
          .backgroundColor(this.running ? $r('app.color.surface_raised') : $r('app.color.accent_tint'))
          .fontColor(this.running ? $r('app.color.text_primary') : $r('app.color.accent'))
          .onClick(() => {
            if (this.running) {
              this.stop();
            } else {
              this.start();
            }
          })
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      // The panel the platform draws into: its surface is what the camera
      // service writes frames to, and the UI never sees them. It keeps its
      // size when idle — a zero-height XComponent has no surface to give — and
      // says so on top of it.
      //
      // The box has the panel's ratio (16:9); the picture has the camera's
      // (`this.aspect`, portrait when held upright). When they differ the frame
      // is **centre-cropped**: the XComponent takes the picture's real ratio and
      // ends up taller than the box, and `.clip(true)` cuts the overhang. Letting
      // the XComponent follow the box instead widened a 9:16 frame ~3x (measured).
      Stack() {
        XComponent({
          id: 'cameraPreview',
          type: XComponentType.SURFACE,
          controller: this.controller,
        })
          .width('100%')
          .aspectRatio(this.aspect)
          .onLoad(() => {
            this.surfaceId = this.controller.getXComponentSurfaceId();
            if (this.wanted && !this.running) {
              this.askPermission();
            }
          })
        // Never removed: taking a sibling out of a Stack rebuilds the
        // XComponent, and a rebuilt one has a new surface — the running session
        // would keep drawing into the old one (a black panel).
        //
        // Opaque, and the full size of the panel, for a second reason: stopping
        // the camera leaves the surface holding its last frame, so a bare text
        // label left a frozen picture sitting under "no preview".
        Column({ space: 4 }) {
          Text('No preview')
            .fontSize(13)
            .fontColor($r('app.color.text_secondary'))
          Text('Frames land here once the camera is on')
            .fontSize(11)
            .fontColor($r('app.color.text_muted'))
        }
        .width('100%')
        .height('100%')
        .justifyContent(FlexAlign.Center)
        .backgroundColor($r('app.color.surface_raised'))
        .opacity(this.running ? 0 : 1)
      }
      .width('100%')
      .aspectRatio(PANEL_ASPECT)
      .borderRadius(8)
      .backgroundColor($r('app.color.surface_raised'))
      .clip(true)

      if (this.status.length > 0) {
        Text(this.status)
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
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
