// The one-line todo input. It owns its own draft text — nothing outside needs to
// know what is half-typed — and hands the finished title up.
//
// The list itself stays with the page, which owns the todos: one owner per piece
// of state, and this is the piece nobody else writes.

@ComponentV2
export struct TodoComposer {
  @Local private draft: string = '';
  @Event onAdd: (title: string) => boolean = (title: string): boolean => false;

  private submit(): void {
    const title = this.draft.trim();
    if (title.length === 0) {
      return;
    }
    if (this.onAdd(title)) {
      this.draft = '';
    }
  }

  build() {
    Row({ space: 8 }) {
      TextInput({ text: this.draft, placeholder: 'Add a todo' })
        .layoutWeight(1)
        .height(40)
        .fontSize(15)
        .fontColor($r('app.color.text_primary'))
        .placeholderColor($r('app.color.text_muted'))
        .placeholderFont({ size: 15 })
        .backgroundColor($r('app.color.surface_raised'))
        .borderRadius(8)
        .padding({ left: 12, right: 12 })
        .onChange((v: string) => {
          this.draft = v;
        })
        .onSubmit(() => {
          this.submit();
        })
      // The page's one filled button: the action that writes something.
      // ButtonType.Normal because the default capsule would make this a pill,
      // and a pill is not a button — it is what every quick mockup looks like.
      Button('Add')
        .type(ButtonType.Normal)
        .height(40)
        .fontSize(14)
        .borderRadius(8)
        .padding({ left: 20, right: 20 })
        .backgroundColor($r('app.color.text_primary'))
        .fontColor($r('app.color.on_primary'))
        .enabled(this.draft.trim().length > 0)
        .onClick(() => {
          this.submit();
        })
    }
    .width('100%')
  }
}
