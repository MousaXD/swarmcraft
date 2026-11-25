package dev.swarmcraft.fabric.mixin;

import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(CommandSourceStack.class)
abstract class OwnerlessCommandSourceMixin {
    @Inject(method = "permissions", at = @At("RETURN"), cancellable = true)
    private void swarmcraft$clampPermissions(CallbackInfoReturnable<PermissionSet> cir) {
        if (OwnerlessPermissionGuard.active()) {
            // Never trust the source's requested elevated level (including
            // withPermission or command compilation contexts).
            PermissionSet original = cir.getReturnValue();
            cir.setReturnValue(((CommandSourceStack) (Object) this).getEntity() instanceof ServerPlayer
                ? permission -> LevelBasedPermissionSet.ALL.hasPermission(permission) && original.hasPermission(permission)
                : PermissionSet.NO_PERMISSIONS);
        }
    }
}
