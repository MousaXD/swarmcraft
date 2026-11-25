package dev.swarmcraft.fabric.mixin;

import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.BaseCommandBlock;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(BaseCommandBlock.class)
abstract class OwnerlessCommandBlockMixin {
    @Inject(method = "performCommand", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyCommandBlock(ServerLevel level, CallbackInfoReturnable<Boolean> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue(false);
        }
    }
}
