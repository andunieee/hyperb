//! A container that gives its one child at most a given width and centers it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk4 as gtk;

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct Clamp {
        pub max_width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "HbClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.obj()
                .first_child()
                .map_or(gtk::SizeRequestMode::ConstantSize, |c| c.request_mode())
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.obj().first_child() else {
                return (0, 0, -1, -1);
            };
            let max = self.max_width.get();
            if orientation == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = child.measure(orientation, for_size);
                (min, nat.min(max).max(min), -1, -1)
            } else {
                let (min_w, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let w = if for_size < 0 {
                    -1
                } else {
                    for_size.min(max).max(min_w)
                };
                let (min, nat, _, _) = child.measure(orientation, w);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let (min_w, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            let w = width.min(self.max_width.get()).max(min_w);
            let x = ((width - w) / 2).max(0);
            let at =
                gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0));
            child.allocate(w, height, baseline, Some(at));
        }
    }
}

glib::wrapper! {
    pub struct Clamp(ObjectSubclass<imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    pub fn new(max_width: i32, child: &impl IsA<gtk::Widget>) -> Self {
        let c: Self = glib::Object::new();
        c.imp().max_width.set(max_width);
        child.set_parent(&c);
        c.set_hexpand(true);
        c
    }
}
